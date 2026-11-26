//! The bridge's own surface: the commands it refuses at once, the folder it names, the
//! threads it started and leaves none of at work, the processes a stopped Claude Code turn
//! leaves none of running, and the editor's `--serve-mcp` argument.
//!
//! Needs kerfd built beside this test: `cargo build -p kerf-runtime --bin kerfd`.
#![expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]

use crate::{poll, send};
use kerf_bridge::{serve_mcp, Bridge, SERVE_MCP};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn kerfd() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/debug")
        .join(format!("kerfd{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "build kerfd first: cargo build -p kerf-runtime --bin kerfd"
    );
    path
}

/// A short project folder, removed at the end, which ends its daemon.
struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("kq-api-{:x}", stamp % 0xffff_ffff));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn what_is_no_command_or_cannot_be_taken_is_refused_and_the_rest_goes_on() {
    let project = Project::new();
    let mut bridge = Bridge::open(&project.0, &kerfd());
    assert_eq!(bridge.directory(), project.0);
    for refused in [
        json!("refresh"),
        json!({"type": "no_such_command"}),
        json!({"type": "edit", "ops": "not a list", "revision": 0}),
        json!({"type": "seek", "position": -1.0}),
        json!({"type": "agent", "text": "   "}),
        json!({"type": "agent_versions", "text": "Tighten the open", "count": 0}),
    ] {
        assert!(!send(&mut bridge, &refused), "took {refused}");
    }
    assert!(send(&mut bridge, &json!({"type": "refresh"})));
    assert!(send(
        &mut bridge,
        &json!({"type": "seek", "position": 0.5, "scrub": true})
    ));

    // The bridge still reaches kerfd.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut events = Vec::new();
    let state = loop {
        if let Some(event) = poll(&mut bridge) {
            if event["type"] == "state" {
                break event;
            }
            events.push(event);
        } else {
            assert!(Instant::now() < deadline, "no state; events: {events:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    assert_eq!(state["snapshot"]["revision"], 0, "{state}");
    // No `scope_signal` was asked for, so there is no signal frame to take.
    assert!(bridge.signal().is_none());
}

#[test]
fn only_the_serve_argument_has_the_editor_serve_its_tools() {
    let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    assert_eq!(SERVE_MCP, "--serve-mcp");
    assert_eq!(serve_mcp(&args(&[])), None);
    assert_eq!(serve_mcp(&args(&["--dir", "/work/film"])), None);
    assert_eq!(serve_mcp(&args(&["/work/film", SERVE_MCP])), None);
    // The argument without `--dir <project>` ends the editor with a failure; it does not
    // start the editor.
    assert_eq!(serve_mcp(&args(&[SERVE_MCP])), Some(1));
    assert_eq!(serve_mcp(&args(&[SERVE_MCP, "--dir"])), Some(1));
}

#[test]
fn the_editor_runs_the_ffmpeg_kerfd_runs() {
    let found = kerf_bridge::ffmpeg_bin();
    // The engine's own, found once: the very same text.
    assert!(std::ptr::eq(
        found,
        kerf_engine::process::ffmpeg::resolve_ffmpeg_bin()
    ));
    // What is found runs; else it is plain `ffmpeg`, to say so as it fails.
    let runs = std::process::Command::new(found)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    assert!(runs || found == "ffmpeg", "{found}");
}

#[test]
fn what_is_answered_at_once_never_holds_the_caller_while_the_events_wait() {
    // The caller is the one that takes the events: an answer waiting for room in their queue
    // on its thread would wait for ever. More answers than the queue holds are asked for here
    // with none taken, from a thread of their own so a block fails the test instead of
    // hanging it.
    let project = Project::new();
    let bridge = Bridge::open(&project.0, &kerfd());
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bridge = bridge;
        for _ in 0..300 {
            send(&mut bridge, &json!({"type": "models"}));
            send(
                &mut bridge,
                &json!({"type": "conversations", "action": "list"}),
            );
        }
        let _ = done.send(bridge);
    });
    let mut bridge = finished
        .recv_timeout(Duration::from_secs(30))
        .expect("a send waited while the events were not taken");
    // Taken, every models answer still comes.
    let mut models = 0;
    let deadline = Instant::now() + Duration::from_secs(60);
    while models < 300 && Instant::now() < deadline {
        match poll(&mut bridge) {
            Some(event) if event["type"] == "models" => models += 1,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    assert_eq!(models, 300);
}

/// Set in the process [`alone`] starts a test in.
const ALONE: &str = "KERF_BRIDGE_API_ALONE";

/// Keep the person's Kerf account and keychain out of a test that waits for every thread of
/// a bridge, the one that asks the keychain for the account among them: a keychain that
/// asks its person first would hold that thread for as long as they took. `test` runs again
/// in a process of its own, this binary started for it alone, with a home of the tests' own
/// and the keychain off. True in that process, where the test goes on; false in this one,
/// once the test has passed there. `env` is set in that process too.
fn alone(test: &str, env: &[(&str, &std::ffi::OsStr)]) -> bool {
    if std::env::var_os(ALONE).is_some() {
        return true;
    }
    let home = std::env::temp_dir().join("kerf-bridge-api-home");
    fs::create_dir_all(&home).unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact"])
        .env(ALONE, "1")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("KERF_SECRETS", "off")
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && said.contains("test result: ok. 1 passed"),
        "{test}, in a process of its own:\n{said}"
    );
    false
}

#[test]
fn a_bridge_let_go_leaves_none_of_the_threads_it_started_at_work() {
    if !alone(
        "api::a_bridge_let_go_leaves_none_of_the_threads_it_started_at_work",
        &[],
    ) {
        return;
    }
    let project = Project::new();
    let mut bridge = Bridge::open(&project.0, &kerfd());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !poll(&mut bridge).is_some_and(|event| event["type"] == "state") {
        assert!(Instant::now() < deadline, "no state");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Its workers are at work, and so are threads that answer at once: more answers than
    // the events' queue holds, with none taken, so that some wait for room in it.
    for _ in 0..300 {
        assert!(send(&mut bridge, &json!({"type": "models"})));
    }
    for command in [
        json!({"type": "preset_library"}),
        json!({"type": "loudness"}),
        json!({"type": "check"}),
        json!({"type": "collab_status"}),
        json!({"type": "conversations", "action": "list"}),
    ] {
        assert!(send(&mut bridge, &command), "{command}");
    }
    let workers = bridge.workers();
    assert!(workers.running() > 0);

    // Let go, each ends on its own: the workers as they see the bridge closed, the rest as
    // they find nobody left to take what they say.
    drop(bridge);
    assert!(
        workers.ended(Duration::from_secs(60)),
        "{} of the threads the bridge started were still at work a minute after it was let go",
        workers.running()
    );
}

/// Stands in for Claude Code, as `claude` beside a `pids` file: it reads the request, starts
/// a long shell command with a child of its own, and another that does not end when asked
/// to, nor does its child, says the turn has begun and waits. Each writes its process id to
/// `pids`; asked to end, the stand-in writes `asked` beside them first.
#[cfg(unix)]
const STAND_IN_CLAUDE: &str = r#"#!/bin/sh
here=$(dirname "$0")
cat >/dev/null
trap 'echo asked >"$here/asked"; exit 143' TERM
sh -c 'sleep 600 & echo $! >>"$1/pids"; wait' long "$here" >/dev/null 2>&1 &
echo $! >>"$here/pids"
sh -c 'trap "" TERM; sleep 600 & echo $! >>"$1/pids"; wait' stubborn "$here" >/dev/null 2>&1 &
echo $! >>"$here/pids"
echo $$ >>"$here/pids"
echo '{"type":"system","subtype":"init","session_id":"stand-in","mcp_servers":[]}'
wait
"#;

/// Whether process `pid` ends within `limit`; a zombie awaiting its new parent counts as
/// ended. `ps` that says nothing, not even why, has found no such process.
#[cfg(unix)]
fn ends_within(pid: &str, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .expect("ps runs");
        let state = String::from_utf8_lossy(&output.stdout);
        let gone = state.trim().is_empty() && output.stderr.is_empty();
        if gone || state.trim_start().starts_with('Z') {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[cfg(unix)]
#[test]
fn a_stopped_claude_code_turn_leaves_nothing_it_started_running() {
    // Claude Code is the stand-in `KERF_CLAUDE_BIN` names, which only a process started with
    // it set reads. The stand-in starts no server: Kerf needs only one named.
    let named = std::env::temp_dir()
        .join(format!("kq-claude-{}", std::process::id()))
        .join("claude");
    if !alone(
        "api::a_stopped_claude_code_turn_leaves_nothing_it_started_running",
        &[
            ("KERF_CLAUDE_BIN", named.as_os_str()),
            ("KERF_MCP_BIN", "kerf-mcp".as_ref()),
        ],
    ) {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let claude = PathBuf::from(std::env::var_os("KERF_CLAUDE_BIN").unwrap());
    let folder = Project(claude.parent().unwrap().to_owned());
    fs::create_dir_all(&folder.0).unwrap();
    fs::write(&claude, STAND_IN_CLAUDE).unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();

    let project = Project::new();
    let mut bridge = Bridge::open(&project.0, &kerfd());
    assert!(send(
        &mut bridge,
        &json!({"type": "agent", "text": "Run the long command", "provider": "claude-code"})
    ));
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let started = loop {
        let said = fs::read_to_string(folder.0.join("pids")).unwrap_or_default();
        let pids: Vec<String> = said.lines().map(str::to_owned).collect();
        if pids.len() == 5 {
            break pids;
        }
        assert!(
            Instant::now() < deadline,
            "the turn did not start all it runs: {said:?}; events: {events:?}"
        );
        events.extend(poll(&mut bridge));
        std::thread::sleep(Duration::from_millis(20));
    };

    assert!(send(&mut bridge, &json!({"type": "abort"})));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match poll(&mut bridge) {
            Some(event) if event["type"] == "agent_state" && event["state"] == "aborted" => break,
            Some(event) => events.push(event),
            None => {
                assert!(
                    Instant::now() < deadline,
                    "the turn did not stop: {events:?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    // Asked to end before anything was killed, and nothing it started outlives the turn.
    assert!(
        folder.0.join("asked").is_file(),
        "Claude Code was not asked to end"
    );
    for pid in &started {
        assert!(
            ends_within(pid, Duration::from_secs(5)),
            "process {pid} of the stopped turn is still running"
        );
    }
}

/// Stands in for Claude Code as `claude` beside a `said.jsonl`: it reads the request and
/// says what that file holds, as Claude Code says a turn on its stream.
#[cfg(unix)]
const REPLAYING_CLAUDE: &str = r#"#!/bin/sh
cat >/dev/null
cat "$(dirname "$0")/said.jsonl"
"#;

/// What reaches the editor of an agent's ask, made in test `test` with `why`: the answer
/// kerf-mcp gives the agent's `make` (its `result`), the card's `agent_ask`, and the ask the
/// conversation keeps. None in the process that runs `test` in a process of its own.
#[cfg(unix)]
fn asked_through_claude_code(
    test: &str,
    why: &str,
) -> Option<(serde_json::Value, serde_json::Value, serde_json::Value)> {
    let named = std::env::temp_dir()
        .join(format!(
            "kq-{}-{}",
            test.replace("::", "-"),
            std::process::id()
        ))
        .join("claude");
    if !alone(
        test,
        &[
            ("KERF_CLAUDE_BIN", named.as_os_str()),
            ("KERF_MCP_BIN", "kerf-mcp".as_ref()),
        ],
    ) {
        return None;
    }
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    fs::write(
        project.0.join("project.kerf"),
        "kerf 1\nproject name=\"Asks\"\n\
         media m1 path=\"shots/open.mp4\" rate=24 dur=12f video=64x64\n\
         \x20 render s1 {out}\n\
         sequence s1 rate=24 size=64x64 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip g1 gen=solid@1 at=0f dur=12f\n",
    )
    .unwrap();
    // What kerf-mcp answers an agent's `make` with, the person being asked: the ask in words.
    let router = kerf_mcp::KerfMcpRouter::new(&project.0, Some(&kerfd())).unwrap();
    router.set_permissions(Some(kerf_mcp::permissions::Permissions::default()));
    let answered = router.call("make", json!({"media": "m1", "why": why}));
    drop(router);
    assert_eq!(answered["result"]["status"], "asked", "{answered}");

    // Claude Code makes the call, and kerf-mcp's answer is the text of its result.
    let claude = PathBuf::from(std::env::var_os("KERF_CLAUDE_BIN").unwrap());
    let folder = Project(claude.parent().unwrap().to_owned());
    fs::create_dir_all(&folder.0).unwrap();
    let said = [
        json!({"type": "system", "subtype": "init", "session_id": "stand-in", "mcp_servers": []}),
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1",
            "name": "mcp__kerf__make", "input": {"media": "m1", "why": why}}]}}),
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1",
            "content": [{"type": "text", "text": answered.to_string()}]}]}}),
        json!({"type": "result", "subtype": "success", "is_error": false,
            "result": "I asked to make m1."}),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect::<String>();
    fs::write(folder.0.join("said.jsonl"), said).unwrap();
    fs::write(&claude, REPLAYING_CLAUDE).unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();

    let mut bridge = Bridge::open(&project.0, &kerfd());
    assert!(send(
        &mut bridge,
        &json!({"type": "agent", "text": "Make m1 wider", "provider": "claude-code"})
    ));
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let kept = loop {
        match poll(&mut bridge) {
            Some(event)
                if event["type"] == "conversation" && event["items"][1]["kind"] == "kerf" =>
            {
                break event;
            }
            Some(event) => events.push(event),
            None => {
                assert!(
                    Instant::now() < deadline,
                    "the turn was not kept: {events:?}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let card = events
        .into_iter()
        .find(|event| event["type"] == "agent_ask")
        .expect("a card was asked for");
    let kept = kept["items"][1]["asks"][0].clone();
    Some((answered["result"].clone(), card, kept))
}

/// An agent's ask reaches the editor in the parts kerf-mcp wrote its words from, on the
/// card's `agent_ask` and in the conversation that keeps it: the editor shows those, and does
/// not read them back out of words worded for the agent.
#[cfg(unix)]
#[test]
fn an_agents_ask_reaches_the_editor_in_its_parts() {
    // A why whose second line reads like a step is the agent's words all the same.
    let why = "the wider look you wanted\n1. keeps the window's light";
    let Some((asked, card, kept)) =
        asked_through_claude_code("api::an_agents_ask_reaches_the_editor_in_its_parts", why)
    else {
        return;
    };
    let steps = &asked["steps"];
    assert!(
        steps.as_array().is_some_and(|steps| !steps.is_empty()),
        "{asked}"
    );
    for ask in [&card, &kept] {
        assert_eq!(ask["media"], "m1", "{ask}");
        assert_eq!(ask["take"], 1, "{ask}");
        assert_eq!(ask["why"], why, "{ask}");
        assert_eq!(&ask["steps"], steps, "{ask}");
        assert_eq!(ask["text"], asked["ask"], "{ask}");
    }
}

/// An ask too long to keep whole reaches the editor as its words do, cut short: its parts
/// as far as they fit, in the order its words have them, and no further.
#[cfg(unix)]
#[test]
fn a_long_ask_reaches_the_editor_cut_short() {
    let why = "the window's light falls across the wall and stays there, ".repeat(100);
    let Some((asked, card, kept)) =
        asked_through_claude_code("api::a_long_ask_reaches_the_editor_cut_short", &why)
    else {
        return;
    };
    for ask in [&card, &kept] {
        let text = ask["text"].as_str().unwrap();
        let cut = text.strip_suffix('…').expect("the words are cut short");
        assert!(asked["ask"].as_str().unwrap().starts_with(cut), "{ask}");
        // The why is as much of it as fits, and nothing comes after it.
        let kept_why = ask["why"].as_str().expect("the why is kept");
        let begun = kept_why.strip_suffix('…').expect("the why is cut short");
        assert!(why.trim().starts_with(begun), "{ask}");
        assert!(kept_why.len() <= 4 * 1024, "{}", kept_why.len());
        assert!(ask["steps"].is_null() && ask["notes"].is_null(), "{ask}");
    }
}
