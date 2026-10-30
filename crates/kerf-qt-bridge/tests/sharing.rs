//! Shared projects through the editor's C ABI with real kerfd daemons: two
//! clones of one bare repository standing in for GitHub, synced by hand
//! and live, and a project started from a preset by the launcher.
//!
//! These need Unix sockets (kerfd), git, and ffmpeg for the media parts.

#![cfg(unix)]

use kerf_collab::git;
use kerf_collab::scaffold::{self, NewProject, Template};
use kerf_model::Catalog;
use kerf_qt_bridge::{
    kerf_bridge_free, kerf_bridge_new, kerf_bridge_poll, kerf_bridge_send, kerf_bridge_string_free,
    kerf_preset_inspect, kerf_project_inspect, kerf_task_free, kerf_task_poll, kerf_task_start,
};
use kerf_runtime::RuntimeClient;
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn kerfd() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/kerfd");
    assert!(
        path.is_file(),
        "build kerfd first: cargo build -p kerf-runtime --bin kerfd"
    );
    path
}

/// Keep the person's git configuration and Kerf account out of the tests.
fn hermetic() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = std::env::temp_dir().join("kerf-qt-bridge-sharing-home");
        fs::create_dir_all(&home).unwrap();
        let empty = home.join("gitconfig");
        fs::write(&empty, "").unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_AUTHOR_NAME", "Test");
        std::env::set_var("GIT_AUTHOR_EMAIL", "test@example.com");
        std::env::set_var("GIT_COMMITTER_NAME", "Test");
        std::env::set_var("GIT_COMMITTER_EMAIL", "test@example.com");
    });
}

/// A short folder: each daemon's socket lives in its project folder.
struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("kq-{label}-{:x}", stamp % 0xffff_ffff));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // Removing the folders orphans the daemons, which then exit.
        let _ = fs::remove_dir_all(&self.0);
    }
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

fn has_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Three seconds of 24 fps picture.
fn footage(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=320x180:rate=24")
        .args([
            "-t",
            "3",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn c_string(value: impl AsRef<str>) -> CString {
    CString::new(value.as_ref()).unwrap()
}

/// Take an owned JSON string from the bridge.
fn owned(pointer: *mut c_char) -> Value {
    assert!(!pointer.is_null());
    // SAFETY: the bridge returned a live, owned, NUL-terminated string.
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: returned exactly once.
    unsafe { kerf_bridge_string_free(pointer) };
    serde_json::from_str(&text).unwrap()
}

/// One project open in the editor, through the C ABI.
struct Editor {
    bridge: *mut c_void,
    events: Vec<Value>,
    seen: usize,
}

impl Editor {
    fn open(directory: &Path) -> Self {
        let directory = c_string(directory.to_string_lossy());
        let binary = c_string(kerfd().to_string_lossy());
        // SAFETY: both strings live for the call.
        let bridge = unsafe { kerf_bridge_new(directory.as_ptr(), binary.as_ptr()) };
        assert!(!bridge.is_null());
        let mut editor = Self {
            bridge,
            events: Vec::new(),
            seen: 0,
        };
        editor.expect("kerfd to be ready", Duration::from_secs(20), |event| {
            event["type"] == "connection" && event["status"] == "ready"
        });
        editor
    }

    fn send(&self, command: Value) {
        let text = c_string(command.to_string());
        // SAFETY: the bridge is live and only this thread uses it.
        assert!(
            unsafe { kerf_bridge_send(self.bridge, text.as_ptr()) },
            "refused {command}"
        );
    }

    fn drain(&mut self) {
        loop {
            // SAFETY: the bridge is live and only this thread uses it.
            let pointer = unsafe { kerf_bridge_poll(self.bridge) };
            if pointer.is_null() {
                break;
            }
            self.events.push(owned(pointer));
        }
    }

    /// The next event after those already taken that `matches`, waiting
    /// up to `timeout`.
    fn wait(&mut self, timeout: Duration, matches: impl Fn(&Value) -> bool) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain();
            while self.seen < self.events.len() {
                let event = &self.events[self.seen];
                self.seen += 1;
                if matches(event) {
                    return Some(event.clone());
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn expect(&mut self, what: &str, timeout: Duration, matches: impl Fn(&Value) -> bool) -> Value {
        match self.wait(timeout, matches) {
            Some(event) => event,
            None => panic!(
                "no event for {what}; events: {}",
                serde_json::to_string_pretty(&self.events[self.events.len().saturating_sub(30)..])
                    .unwrap()
            ),
        }
    }

    /// Wait for a sync or other collaboration command to finish; its
    /// report.
    fn finished(&mut self, what: &str) -> Value {
        let event = self.expect(what, Duration::from_secs(60), |event| {
            event["type"] == "collab_sync"
                && (event["stage"] == "done" || event["stage"] == "failed")
        });
        assert_eq!(event["stage"], "done", "{what}: {event}");
        event["report"].clone()
    }

    fn sync(&mut self, what: &str) -> Value {
        self.send(json!({"type": "collab_sync"}));
        self.finished(what)
    }

    /// A state, published at any time so far or within `timeout`, whose
    /// document contains `text`.
    fn document_with(&mut self, what: &str, text: &str, timeout: Duration) -> Value {
        let matches = |event: &Value| {
            event["type"] == "state"
                && event["snapshot"]["document"]
                    .as_str()
                    .is_some_and(|document| document.contains(text))
        };
        self.drain();
        if let Some(event) = self.events.iter().rev().find(|event| matches(event)) {
            return event.clone();
        }
        self.expect(what, timeout, matches)
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        // SAFETY: made by kerf_bridge_new and freed once.
        unsafe { kerf_bridge_free(self.bridge) };
    }
}

fn edit(directory: &Path, key: &str, script: &str) {
    let client = RuntimeClient::new(directory).unwrap();
    let revision = client.get_state().unwrap().revision;
    client
        .apply_script_request("ui", key, script, revision)
        .unwrap();
}

#[test]
fn two_editors_share_a_project_by_hand_and_live() {
    hermetic();
    let root = Root::new("sync");
    let remote = root.0.join("remote.git");
    fs::create_dir_all(&remote).unwrap();
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let alice_dir = scaffold::create(
        &NewProject {
            name: "Film".into(),
            location: root.0.join("a"),
            template: Template::Standard,
            rate: Some("24".into()),
            git: true,
            remote: Some(remote.to_string_lossy().into_owned()),
            github: None,
        },
        Catalog::builtin(),
    )
    .unwrap()
    .path;
    let bob_dir = git::clone(&remote.to_string_lossy(), &root.0.join("b")).unwrap();
    let mut alice = Editor::open(&alice_dir);
    let mut bob = Editor::open(&bob_dir);

    alice.send(json!({"type": "collab_status"}));
    let status = alice.expect("the status", Duration::from_secs(10), |event| {
        event["type"] == "collab"
    });
    assert_eq!(status["status"]["repository"], true, "{status}");
    assert_eq!(status["status"]["branch"], "main");
    assert_eq!(status["status"]["live"], false);
    assert_eq!(status["status"]["remote"], json!(remote.to_string_lossy()));
    // Opening a shared project makes plain git merge it structurally too.
    assert!(sh(&alice_dir, &["config", "merge.kerf.driver"]).contains("merge-driver"));

    // A sync commits, merges, and pushes.
    edit(&alice_dir, "a1", "marker add at=0f as=intro");
    let report = alice.sync("Alice's first sync");
    assert!(report["committed"].is_string(), "{report}");
    assert_eq!(report["pushed"], true, "{report}");
    assert_eq!(
        sh(&remote, &["log", "-1", "--format=%s", "main"]),
        sh(&alice_dir, &["log", "-1", "--format=%s"])
    );

    let report = bob.sync("Bob's first sync");
    assert_eq!(report["incoming"].as_array().unwrap().len(), 1, "{report}");
    assert!(
        !report["summary"].as_array().unwrap().is_empty(),
        "{report}"
    );
    bob.document_with(
        "Alice's marker at Bob's",
        "marker intro ",
        Duration::from_secs(10),
    );

    // Live sync brings Alice's next edit to Bob without asking...
    bob.send(json!({"type": "collab_live", "enabled": true, "interval": 2}));
    bob.expect("live sync on", Duration::from_secs(10), |event| {
        event["type"] == "collab" && event["status"]["live"] == true
    });
    edit(&alice_dir, "a2", "marker add at=24f as=outro");
    alice.sync("Alice's second sync");
    bob.document_with(
        "Alice's second marker, live",
        "marker outro ",
        Duration::from_secs(30),
    );

    // ...and takes Bob's own edit to Alice a few seconds after he makes it.
    edit(&bob_dir, "b1", "marker add at=48f as=bobs");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        bob.drain();
        if sh(&remote, &["show", "main:project.kerf"]).contains("marker bobs ") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Bob's edit was not pushed by live sync: {:?}",
            bob.events
                .iter()
                .filter(|event| event["type"] == "collab_sync" || event["type"] == "error")
                .map(|event| event.to_string())
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    alice.sync("Alice pulls Bob's edit");
    alice.document_with(
        "Bob's marker at Alice's",
        "marker bobs ",
        Duration::from_secs(10),
    );

    // A live sync that keeps failing says so once, and leaves no stage
    // hanging, though there is an edit to commit each time.
    sh(
        &bob_dir,
        &["remote", "set-url", "origin", "/nonexistent/remote.git"],
    );
    edit(&bob_dir, "b2", "marker add at=72f as=offline");
    std::thread::sleep(Duration::from_secs(9));
    bob.drain();
    let syncs: Vec<&Value> = bob
        .events
        .iter()
        .filter(|event| event["type"] == "collab_sync")
        .collect();
    let failures = syncs
        .iter()
        .filter(|event| event["stage"] == "failed")
        .count();
    assert_eq!(failures, 1, "{syncs:?}");
    assert_eq!(syncs.last().unwrap()["stage"], "failed", "{syncs:?}");
    // Back online, the next cycle sends the edit.
    sh(
        &bob_dir,
        &["remote", "set-url", "origin", &remote.to_string_lossy()],
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while !sh(&remote, &["show", "main:project.kerf"]).contains("marker offline ") {
        bob.drain();
        assert!(Instant::now() < deadline, "live sync did not recover");
        std::thread::sleep(Duration::from_millis(200));
    }
    bob.send(json!({"type": "collab_live", "enabled": false}));

    // Branches: commit, switch, and come back.
    alice.send(json!({"type": "collab_branch", "name": "alice/color", "create": true}));
    let report = alice.finished("a new branch");
    assert_eq!(report["branch"], "alice/color", "{report}");
    assert_eq!(git::branch(&alice_dir).as_deref(), Some("alice/color"));
    alice.send(json!({"type": "collab_commit", "message": "Nothing yet"}));
    let report = alice.finished("an empty commit");
    assert!(report["committed"].is_null(), "{report}");

    // Media imported into a shared project is copied into media/ by
    // content, and travels with the next sync.
    if has_ffmpeg() {
        let outside = root.0.join("Downloads/shot.mp4");
        footage(&outside);
        alice.send(json!({"type": "import", "path": outside}));
        let state = alice.document_with(
            "the imported copy",
            "path=\"media/shot-",
            Duration::from_secs(30),
        );
        let document = state["snapshot"]["document"].as_str().unwrap();
        let start = document.find("media/shot-").unwrap();
        let copy = &document[start..document[start..].find('"').unwrap() + start];
        assert!(alice_dir.join(copy).is_file(), "{copy}");
        assert_eq!(
            fs::read(alice_dir.join(copy)).unwrap(),
            fs::read(&outside).unwrap()
        );
        let report = alice.sync("Alice shares the media");
        assert_eq!(report["pushed"], true, "{report}");
        assert!(sh(&remote, &["ls-tree", "-r", "--name-only", "alice/color"]).contains(copy));
    }
}

#[test]
fn the_launcher_starts_a_shared_project_from_a_preset() {
    if !has_ffmpeg() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    hermetic();
    let root = Root::new("preset");
    let shot = root.0.join("footage/shot.mp4");
    footage(&shot);
    let program = kerf_runtime::parse_document(
        "kerf 1\nproject name=\"Source\"\n\
         media m1 path=\"/elsewhere/a.mov\" rate=24 dur=96f video=320x180\n\
         sequence s1 rate=24 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=0f at=0f dur=48f\n\
         \x20   clip c2 media=m1 src=48f at=48f dur=24f\n\
         \x20 track S1 subtitle\n\
         \x20   clip t1 caption=\"Hello\" at=0f dur=24f\n",
    )
    .unwrap();
    let sequence = program.sequences[0].id.clone();
    let document =
        kerf_lang::preset::export(&program, Catalog::builtin(), &sequence, "Two Cuts").unwrap();
    let preset_path = root.0.join("Two Cuts.kerfpreset");
    fs::write(&preset_path, &document).unwrap();
    let slots: Vec<String> = kerf_lang::preset::parse(&document, Catalog::builtin())
        .unwrap()
        .pattern
        .media
        .iter()
        .map(|slot| slot.id.to_string())
        .collect();
    assert_eq!(slots.len(), 2);

    // The launcher shows which slots the file can fill.
    let request = c_string(json!({"path": preset_path, "files": [shot]}).to_string());
    // SAFETY: a live string for the call.
    let inspected = owned(unsafe { kerf_preset_inspect(request.as_ptr()) });
    assert_eq!(inspected["name"], "Two Cuts");
    assert_eq!(inspected["files"][0]["fits"], json!(slots), "{inspected}");
    assert_eq!(inspected["files"][0]["video"], true);

    let remote = root.0.join("remote.git");
    fs::create_dir_all(&remote).unwrap();
    sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
    let request = json!({
        "type": "create", "name": "From Preset", "location": root.0.join("projects"),
        "template": "standard", "rate": "24", "git": true, "remote": remote,
        "kerfd": kerfd(),
        "preset": {"path": preset_path, "bindings": {
            slots[0].clone(): {"file": shot},
            slots[1].clone(): {"file": shot, "from": "2s"},
        }, "texts": {"t1": "Bonjour"}},
    });
    let request = c_string(request.to_string());
    // SAFETY: a live string for the call.
    let task = unsafe { kerf_task_start(request.as_ptr()) };
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        // SAFETY: a live task.
        let status = owned(unsafe { kerf_task_poll(task) });
        if status["status"] != "running" {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the task did not finish: {status}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    // SAFETY: made by kerf_task_start and freed once.
    unsafe { kerf_task_free(task) };
    assert_eq!(status["status"], "done", "{status}");
    let project = PathBuf::from(status["path"].as_str().unwrap());
    assert!(project.join("presets/Two Cuts.kerfpreset").is_file());
    let copies: Vec<String> = fs::read_dir(project.join("media"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(copies.len(), 1, "one file fills both slots: {copies:?}");
    assert!(copies[0].starts_with("shot-") && copies[0].ends_with(".mp4"));

    let (_, document) = RuntimeClient::new(&project)
        .unwrap()
        .get_document(false)
        .unwrap();
    assert!(
        document.contains(&format!("path=\"media/{}\"", copies[0])),
        "{document}"
    );
    assert!(document.contains("caption=\"Bonjour\""), "{document}");
    assert!(document.contains("name=\"Two Cuts\""), "{document}");
    let parsed = kerf_runtime::parse_document(&document).unwrap();
    let clips: usize = parsed.sequences[0]
        .tracks
        .iter()
        .map(|track| track.clips.len())
        .sum();
    assert_eq!(
        clips, 3,
        "the pattern replaced the imported clip: {document}"
    );

    assert_eq!(
        sh(&project, &["log", "-1", "--format=%s"]),
        "Start from preset Two Cuts"
    );
    assert_eq!(
        sh(&remote, &["log", "-1", "--format=%s", "main"]),
        "Start from preset Two Cuts"
    );
    assert_eq!(sh(&project, &["status", "--porcelain"]), "");

    let request = c_string(json!({"path": project}).to_string());
    // SAFETY: a live string for the call.
    let inspected = owned(unsafe { kerf_project_inspect(request.as_ptr()) });
    assert_eq!(inspected["repository"], true);
    assert_eq!(inspected["presets"], 1);
    assert_eq!(inspected["name"], "From Preset");
}
