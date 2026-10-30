//! Two people editing one shared project through real kerfd daemons, a
//! bare repository standing in for GitHub, and the structural merge driver.

#![cfg(unix)]

use kerf_collab::git;
use kerf_collab::scaffold::{self, NewProject, Template};
use kerf_runtime::RuntimeClient;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

fn hermetic() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let empty = std::env::temp_dir().join("kerf-runtime-empty-gitconfig");
        fs::write(&empty, "").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_AUTHOR_NAME", "Test");
        std::env::set_var("GIT_AUTHOR_EMAIL", "test@example.com");
        std::env::set_var("GIT_COMMITTER_NAME", "Test");
        std::env::set_var("GIT_COMMITTER_EMAIL", "test@example.com");
        std::env::set_var("KERF_MERGE_DRIVER", env!("CARGO_BIN_EXE_kerfd"));
    });
}

fn root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // Short: the daemon's socket lives in the project folder.
    let root = std::env::temp_dir().join(format!("kc-{label}-{:x}", stamp % 0xffff_ffff));
    fs::create_dir_all(&root).unwrap();
    root
}

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Team {
    root: PathBuf,
    alice: PathBuf,
    bob: PathBuf,
}

impl Drop for Team {
    fn drop(&mut self) {
        // Removing the folders orphans the daemons, which then exit.
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn team(label: &str) -> Team {
    hermetic();
    let root = root(label);
    let remote = root.join("remote.git");
    fs::create_dir_all(&remote).unwrap();
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let created = scaffold::create(
        &NewProject {
            name: "Film".into(),
            location: root.join("a"),
            template: Template::Standard,
            rate: Some("24".into()),
            git: true,
            remote: Some(remote.to_string_lossy().into_owned()),
            github: None,
        },
        kerf_model::Catalog::builtin(),
    )
    .unwrap();
    let bob = git::clone(&remote.to_string_lossy(), &root.join("b")).unwrap();
    Team {
        alice: created.path,
        bob,
        root,
    }
}

fn daemon(directory: &Path) -> RuntimeClient {
    RuntimeClient::connect_or_spawn(directory, Some(Path::new(env!("CARGO_BIN_EXE_kerfd"))))
        .expect("spawn kerfd")
}

fn edit(client: &RuntimeClient, key: &str, script: &str) {
    let revision = client.get_state().unwrap().revision;
    client
        .apply_script_request("ui", key, script, revision)
        .unwrap();
}

/// Commit, merge what the remote has, and push, as the editor's sync does.
fn sync(directory: &Path, client: &RuntimeClient, key: &str) -> serde_json::Value {
    git::fetch(directory).unwrap();
    let (_, report) = client.collab_integrate("ui", key, None).unwrap();
    git::push(directory).unwrap();
    report
}

#[test]
fn edits_made_at_the_same_time_on_two_machines_both_survive_a_sync() {
    let team = team("both");
    let alice = daemon(&team.alice);
    let bob = daemon(&team.bob);

    edit(&alice, "a1", "marker add at=0f as=intro");
    let report = sync(&team.alice, &alice, "sync-a1");
    assert!(report["committed"].is_string(), "{report}");
    let message = sh(&team.alice, &["log", "-1", "--format=%B"]);
    assert!(message.contains("intro"), "{message}");

    // Bob added his own marker before he saw Alice's.
    edit(&bob, "b1", "marker add at=48f as=outro");
    let report = sync(&team.bob, &bob, "sync-b1");
    assert_eq!(report["incoming"].as_array().unwrap().len(), 1, "{report}");
    assert_eq!(report["fastForward"], false);
    let state = bob.get_state().unwrap();
    assert!(
        state.document.contains("marker intro "),
        "{}",
        state.document
    );
    assert!(
        state.document.contains("marker outro "),
        "{}",
        state.document
    );
    assert!(
        !state.can_undo,
        "a sync that changed the project clears history"
    );
    assert_eq!(state.projection_status, "current");
    assert!(!git::status(&team.bob).unwrap().changed);

    let report = sync(&team.alice, &alice, "sync-a2");
    assert_eq!(report["fastForward"], true, "{report}");
    let state = alice.get_state().unwrap();
    assert!(state.document.contains("marker intro "));
    assert!(state.document.contains("marker outro "));
    assert_eq!(
        fs::read_to_string(team.alice.join("project.kerf")).unwrap(),
        fs::read_to_string(team.bob.join("project.kerf")).unwrap()
    );
}

#[test]
fn a_sync_with_nothing_new_changes_nothing() {
    let team = team("idle");
    let alice = daemon(&team.alice);
    let before = alice.get_state().unwrap();
    let report = sync(&team.alice, &alice, "idle-1");
    assert!(report["committed"].is_null(), "{report}");
    let after = alice.get_state().unwrap();
    assert_eq!(before.revision, after.revision);
    // The same request again replays its receipt.
    let (_, replayed) = alice.collab_integrate("ui", "idle-1", None).unwrap();
    assert!(replayed.is_object());
}

#[test]
fn a_hand_edited_project_file_is_not_synced_until_it_is_settled() {
    let team = team("hand");
    let alice = daemon(&team.alice);
    alice.get_state().unwrap();
    let path = team.alice.join("project.kerf");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{text}\n# edited by hand\n  ")).unwrap();
    fs::write(&path, text.replace("name=\"Film\"", "name=\"Hand\"")).unwrap();
    let error = alice.collab_integrate("ui", "hand-1", None).unwrap_err();
    assert_eq!(error.code, "PROJECTION_DIVERGED");
    let revision = alice.get_state().unwrap().revision;
    let state = alice
        .import_projection_request("ui", "hand-load", revision)
        .unwrap();
    assert!(state.document.contains("name=\"Hand\""));
    alice
        .collab_integrate("ui", "hand-2", Some("Rename"))
        .unwrap();
    assert_eq!(sh(&team.alice, &["log", "-1", "--format=%s"]), "Rename");
}

#[test]
fn branches_hold_their_own_edits() {
    let team = team("branch");
    let alice = daemon(&team.alice);
    let (state, report) = alice
        .collab_switch("ui", "to-color", "alice/color", true, None)
        .unwrap();
    assert_eq!(report["branch"], "alice/color");
    let base_revision = state.revision;
    edit(&alice, "c1", "marker add at=12f as=grade");
    let (state, report) = alice
        .collab_switch("ui", "to-main", "main", false, None)
        .unwrap();
    assert!(report["committed"].is_string(), "{report}");
    assert!(!state.document.contains("marker grade "));
    assert!(state.revision > base_revision);
    let (state, _) = alice
        .collab_switch("ui", "back", "alice/color", false, None)
        .unwrap();
    assert!(state.document.contains("marker grade "));
    assert_eq!(git::branch(&team.alice).as_deref(), Some("alice/color"));
}

#[test]
fn kerfd_merges_project_documents_as_git_asks() {
    hermetic();
    let root = root("driver");
    let base =
        "kerf 1\nproject name=\"P\"\n\nsequence s1 name=\"P\" rate=24 size=1920x1080 audio=48000\n";
    let ours = format!("{base}  marker m1 at=00:00:00:00\n");
    let theirs = format!("{base}  marker m2 at=00:00:01:00\n");
    for (name, text) in [
        ("base", base),
        ("ours", ours.as_str()),
        ("theirs", theirs.as_str()),
    ] {
        fs::write(root.join(name), text).unwrap();
    }
    let status = Command::new(env!("CARGO_BIN_EXE_kerfd"))
        .current_dir(&root)
        .args(["merge-driver", "base", "ours", "theirs", "project.kerf"])
        .status()
        .unwrap();
    assert!(status.success());
    let merged = fs::read_to_string(root.join("ours")).unwrap();
    assert!(merged.contains("marker m1 "), "{merged}");
    assert!(merged.contains("marker m2 "), "{merged}");
    fs::write(root.join("broken"), "not a kerf document").unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_kerfd"))
        .current_dir(&root)
        .args(["merge-driver", "base", "broken", "theirs"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1));
    let _ = fs::remove_dir_all(&root);
}
