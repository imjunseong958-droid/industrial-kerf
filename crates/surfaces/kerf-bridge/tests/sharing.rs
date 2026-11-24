//! Shared projects through the editor's bridge with real kerfd daemons: two
//! clones of one bare repository standing in for GitHub, synced by hand
//! and live, and a project started from a preset by the launcher.
//!
//! These need Unix sockets (kerfd), git, and ffmpeg for the media parts.

#![cfg(unix)]

use kerf_bridge::{preset_inspect, project_inspect, Bridge, Task, TaskDone, TaskStatus};
use kerf_collab::git;
use kerf_collab::scaffold::{self, NewProject, Template};
use kerf_model::Catalog;
use kerf_runtime::RuntimeClient;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Send `command` as the editor writes it, in JSON. One that does not read as a command is
/// refused, as one the bridge cannot take is.
fn send(bridge: &mut Bridge, command: &Value) -> bool {
    serde_json::from_value::<kerf_bridge::Command>(command.clone())
        .is_ok_and(|command| bridge.send(command))
}

/// The next event, in JSON, as the editor reads it.
fn poll(bridge: &mut Bridge) -> Option<Value> {
    bridge
        .poll()
        .map(|event| serde_json::to_value(event).expect("an event writes as JSON"))
}

fn kerfd() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    assert!(
        path.is_file(),
        "build kerfd first: cargo build -p kerf-runtime --bin kerfd"
    );
    path
}

/// Set in the process [`hermetic`] starts a test in.
const HERMETIC: &str = "KERF_BRIDGE_SHARING_HERMETIC";

/// Keep the person's git configuration and Kerf account out of the tests: `test` runs again
/// in a process of its own, this binary started for it alone, with a home and a git
/// configuration of the tests' own in its environment. The bridge reads the home in many
/// places (the Kerf account, the models, the caches) and the kerfd and git it starts read
/// both, so the environment it starts with is what reaches them all. True in that process,
/// where the test goes on; false in this one, once the test has passed there.
fn hermetic(test: &str) -> bool {
    if std::env::var_os(HERMETIC).is_some() {
        return true;
    }
    let home = std::env::temp_dir().join("kerf-bridge-sharing-home");
    fs::create_dir_all(&home).unwrap();
    let empty = home.join("gitconfig");
    fs::write(&empty, "").unwrap();
    let out = Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact"])
        .env(HERMETIC, "1")
        .env("HOME", &home)
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // DIAGNOSTIC (temporary): pass the timing lines on whether or not the test passed.
    {
        use std::io::Write;
        let lines: String = said
            .lines()
            .filter(|line| line.starts_with("kerf-timing"))
            .map(|line| format!("[{test}] {line}\n"))
            .collect();
        let _ = std::io::stderr().write_all(lines.as_bytes());
    }
    assert!(
        out.status.success() && said.contains("test result: ok. 1 passed"),
        "{test}, in a process of its own:\n{said}"
    );
    false
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

/// One project open in the editor, through its bridge.
struct Editor {
    bridge: Bridge,
    events: Vec<Value>,
    seen: usize,
}

impl Editor {
    fn open(directory: &Path) -> Self {
        let mut editor = Self {
            bridge: Bridge::open(directory, &kerfd()),
            events: Vec::new(),
            seen: 0,
        };
        editor.expect("kerfd to be ready", Duration::from_secs(20), |event| {
            event["type"] == "connection" && event["status"] == "ready"
        });
        editor
    }

    fn send(&mut self, command: Value) {
        assert!(send(&mut self.bridge, &command), "refused {command}");
    }

    fn drain(&mut self) {
        while let Some(event) = poll(&mut self.bridge) {
            self.events.push(event);
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

fn edit(directory: &Path, key: &str, script: &str) {
    let client = RuntimeClient::new(directory).unwrap();
    let revision = client.get_state().unwrap().revision;
    client
        .apply_script_request("ui", key, script, revision)
        .unwrap();
}

#[test]
fn two_editors_share_a_project_by_hand_and_live() {
    if !hermetic("two_editors_share_a_project_by_hand_and_live") {
        return;
    }
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
    git::diag_line("test: opening alice");
    let mut alice = Editor::open(&alice_dir);
    git::diag_line("test: alice ready, opening bob");
    let mut bob = Editor::open(&bob_dir);
    git::diag_line("test: bob ready, asking alice for the status");

    alice.send(json!({"type": "collab_status"}));
    let status = alice.expect("the status", Duration::from_secs(10), |event| {
        event["type"] == "collab"
    });
    git::diag_line("test: alice's status came");
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
    if !hermetic("the_launcher_starts_a_shared_project_from_a_preset") {
        return;
    }
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
    let inspected = preset_inspect(&preset_path, std::slice::from_ref(&shot)).unwrap();
    assert_eq!(inspected.name, "Two Cuts");
    assert_eq!(inspected.files[0].fits, slots, "{inspected:?}");
    assert!(inspected.files[0].video);

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
    let task = Task::start(request);
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        let status = task.status();
        if !matches!(status, TaskStatus::Running { .. }) {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the task did not finish: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let TaskStatus::Done(TaskDone::Project(created)) = status else {
        panic!("{status:?}");
    };
    let project = created.path;
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

    let inspected = project_inspect(&project);
    assert!(inspected.repository);
    assert_eq!(inspected.presets, 1);
    assert_eq!(inspected.name, "From Preset");
}

// DIAGNOSTIC (temporary): how long the editor waits for its first `collab` status, over
// several rounds, and how long each tool probe takes with the runner's home and with the
// tests' own. Only exit codes and times are written, never what a probe prints.
fn diag_probes(label: &str) {
    let timed = |program: &str, args: &[&str]| {
        let start = Instant::now();
        let code = Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .and_then(|status| status.code());
        (code, start.elapsed().as_secs_f64())
    };
    for _ in 0..4 {
        for (program, args) in [
            ("gh", &["--version"][..]),
            ("git", &["--version"][..]),
            ("git", &["lfs", "version"][..]),
            ("gh", &["auth", "token"][..]),
        ] {
            let (code, secs) = timed(program, args);
            git::diag_line(&format!(
                "diag[{label}] `{program} {}` exit {code:?} in {secs:.3}s",
                args.join(" ")
            ));
        }
    }
    let start = Instant::now();
    let threads: Vec<_> = (0..3)
        .map(|_| std::thread::spawn(move || timed("gh", &["auth", "token"])))
        .collect();
    let each: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    git::diag_line(&format!(
        "diag[{label}] three `gh auth token` at once: {each:?}, {:.3}s in all",
        start.elapsed().as_secs_f64()
    ));
    if cfg!(target_os = "macos") {
        // The lookup gh makes when no account is configured, without -w: exit code and
        // time only.
        let (code, secs) = timed(
            "/usr/bin/security",
            &["find-generic-password", "-s", "gh:github.com"],
        );
        git::diag_line(&format!(
            "diag[{label}] keychain lookup for gh:github.com exit {code:?} in {secs:.3}s"
        ));
        let (code, secs) = timed("/usr/bin/security", &["list-keychains"]);
        git::diag_line(&format!(
            "diag[{label}] `security list-keychains` exit {code:?} in {secs:.3}s"
        ));
    }
}

#[test]
fn diag_collab_status_latency() {
    if std::env::var_os(HERMETIC).is_none() {
        diag_probes("runner home");
    }
    if !hermetic("diag_collab_status_latency") {
        return;
    }
    diag_probes("tests' home");
    for round in 0..6 {
        let root = Root::new("diag");
        let remote = root.0.join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        sh(&remote, &["init", "-q", "--bare", "-b", "main"]);
        let alice_dir = git::diag_timed(&format!("diag round {round}: scaffold::create"), || {
            scaffold::create(
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
            .path
        });
        let bob_dir = git::clone(&remote.to_string_lossy(), &root.0.join("b")).unwrap();
        let opened = Instant::now();
        let mut alice = Editor::open(&alice_dir);
        let early = alice.events.iter().any(|event| event["type"] == "collab");
        let _bob = Editor::open(&bob_dir);
        let sent = Instant::now();
        alice.send(json!({"type": "collab_status"}));
        let got = alice.wait(Duration::from_secs(60), |event| event["type"] == "collab");
        git::diag_line(&format!(
            "diag round {round}: both open in {:.3}s; a status before ready: {early}; \
             answered: {} after {:.3}s ({:.3}s after alice opened)",
            (sent - opened).as_secs_f64(),
            got.is_some(),
            sent.elapsed().as_secs_f64(),
            opened.elapsed().as_secs_f64()
        ));
    }
}
