//! The review log through the editor's C ABI and a real kerfd: notes the
//! editor writes, notes another client writes, and QC findings kept as
//! comments.
//!
//! Needs kerfd built beside this test: `cargo build -p kerf-runtime --bin kerfd`.

use kerf_qt_bridge::{
    kerf_bridge_free, kerf_bridge_new, kerf_bridge_poll, kerf_bridge_send, kerf_bridge_string_free,
    kerf_fx_catalog_all,
};
use kerf_runtime::{Author, NewComment, RuntimeClient};
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
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
    fn new(tag: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("kq-{tag}-{:x}", stamp % 0xffff_ffff));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Editor {
    bridge: *mut c_void,
    events: Vec<Value>,
}

impl Editor {
    fn open(directory: &Path) -> Self {
        let directory = CString::new(directory.to_string_lossy().as_ref()).unwrap();
        let binary = CString::new(kerfd().to_string_lossy().as_ref()).unwrap();
        // SAFETY: both strings live for the call.
        let bridge = unsafe { kerf_bridge_new(directory.as_ptr(), binary.as_ptr()) };
        assert!(!bridge.is_null());
        Self {
            bridge,
            events: Vec::new(),
        }
    }

    fn send(&self, command: Value) -> bool {
        let text = CString::new(command.to_string()).unwrap();
        // SAFETY: the bridge is live and only this thread uses it.
        unsafe { kerf_bridge_send(self.bridge, text.as_ptr()) }
    }

    /// The next event that `matches`, among those not yet taken.
    fn expect(&mut self, what: &str, matches: impl Fn(&Value) -> bool) -> Value {
        if let Some(index) = self.events.iter().position(&matches) {
            return self.events.remove(index);
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            // SAFETY: the bridge is live and only this thread uses it.
            let pointer = unsafe { kerf_bridge_poll(self.bridge) };
            if pointer.is_null() {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            let event = take(pointer);
            if matches(&event) {
                return event;
            }
            self.events.push(event);
        }
        let recent = &self.events[self.events.len().saturating_sub(8)..];
        panic!("no event for {what}; recent events: {recent:?}");
    }

    fn comments(&mut self, what: &str, matches: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        self.expect(what, |event| {
            event["type"] == "comments"
                && event["items"]
                    .as_array()
                    .is_some_and(|items| matches(items))
        })["items"]
            .as_array()
            .cloned()
            .unwrap()
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        // SAFETY: freed exactly once, by its only user.
        unsafe { kerf_bridge_free(self.bridge) };
    }
}

fn take(pointer: *mut c_char) -> Value {
    // SAFETY: the bridge returned a live, owned, NUL-terminated string.
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: returned exactly once.
    unsafe { kerf_bridge_string_free(pointer) };
    serde_json::from_str(&text).unwrap()
}

#[test]
fn notes_from_the_editor_and_from_agents_meet_in_one_review_log() {
    let project = Project::new("review");
    let mut editor = Editor::open(&project.0);
    editor.expect("kerfd to be ready", |event| {
        event["type"] == "connection" && event["status"] == "ready"
    });
    // The list comes once on connecting, empty in a new project.
    assert!(editor
        .comments("the first list", |items| items.is_empty())
        .is_empty());

    // The editor's note is the person's.
    assert!(editor.send(json!({
        "type": "comment_add", "id": "c-qt-1", "body": "Open on the road, not the sky",
        "at": 3.0, "tags": ["director"],
        "drawing": [{"tool": "ellipse", "points": [[0.2, 0.2], [0.4, 0.5]], "color": "#ededed"}],
    })));
    let items = editor.comments("the editor's note", |items| items.len() == 1);
    assert_eq!(items[0]["id"], "c-qt-1");
    assert_eq!(items[0]["author"]["kind"], "person");
    assert_eq!(items[0]["drawing"][0]["tool"], "ellipse");
    assert!(project.0.join("review/comments.jsonl").is_file());

    // An MCP agent writes through its own client; the state poll carries
    // the new review revision and the bridge sends the list again.
    let agent = RuntimeClient::new(&project.0).unwrap();
    let mut note = NewComment::new(
        Author {
            kind: "mcp".into(),
            name: "Claude Code".into(),
            id: Some("claude-code".into()),
            role: Some("MCP · via Kerf".into()),
        },
        "Moved the logo reveal to the snare",
    );
    note.parent = Some("c-qt-1".into());
    agent.add_comment("mcp", note).unwrap();
    let items = editor.comments("the agent's note, live", |items| items.len() == 2);
    assert_eq!(items[1]["author"]["name"], "Claude Code");
    assert_eq!(items[1]["parent"], "c-qt-1");

    // Resolving it from the editor.
    assert!(editor.send(json!({"type": "comment_set", "id": "c-qt-1", "resolved": true})));
    let items = editor.comments("the resolved note", |items| {
        items.iter().any(|item| item["resolved"] == true)
    });
    assert_eq!(items[0]["resolvedBy"]["kind"], "person");

    // A refusal names the command, so the composer can say why.
    assert!(editor.send(json!({"type": "comment_set", "id": "c-missing", "deleted": true})));
    let refused = editor.expect("the refusal", |event| {
        event["type"] == "error" && event["command"] == "comment_set"
    });
    assert_eq!(refused["code"], "COMMENT_NOT_FOUND");
    assert_eq!(refused["id"], "c-missing");

    // Listing again sends the same list.
    assert!(editor.send(json!({"type": "comment_list"})));
    editor.comments("the list again", |items| items.len() == 2);
}

#[test]
fn a_check_reports_its_rows_and_keeps_failing_findings_as_qc_comments() {
    let project = Project::new("check");
    // A caption too fast to read and a clip whose media is missing.
    fs::write(
        project.0.join("project.kerf"),
        "kerf 1\nproject name=\"Check\"\n\
         media m1 path=\"media/gone.mov\" rate=24 dur=96f video=320x180\n\
         sequence s1 rate=24 size=320x180 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=0f at=0f dur=48f\n\
         \x20 track S1 subtitle\n\
         \x20   clip t1 caption=\"Far too many words to read in this little time\" at=0f dur=12f\n",
    )
    .unwrap();
    let mut editor = Editor::open(&project.0);
    editor.expect("kerfd to be ready", |event| {
        event["type"] == "connection" && event["status"] == "ready"
    });
    assert!(editor.send(json!({"type": "check"})));
    editor.expect("the check to start", |event| {
        event["type"] == "check" && event["status"] == "running"
    });
    let done = editor.expect("the check to finish", |event| {
        event["type"] == "check" && event["status"] != "running"
    });
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["valid"], false);
    let rows = done["summary"]["rows"].as_array().unwrap();
    assert!(rows.len() >= 8, "every row, passing or not: {rows:?}");
    let row = |code: &str| rows.iter().find(|row| row["code"] == code).unwrap().clone();
    assert_eq!(row("media")["passed"], false);
    assert!(row("media")["commentId"].is_string());
    assert_eq!(row("caption.reading_speed")["passed"], false);
    assert!(row("text.safe_area").get("commentId").is_none());

    let items = editor.comments("the qc comments", |items| {
        items
            .iter()
            .filter(|item| item["author"]["kind"] == "qc")
            .count()
            >= 2
    });
    let media = items
        .iter()
        .find(|item| item["id"] == row("media")["commentId"])
        .unwrap();
    assert_eq!(media["blocking"], true);
    assert_eq!(media["qcKey"], "qc:missing_media:m1");
    let speed = items
        .iter()
        .find(|item| item["id"] == row("caption.reading_speed")["commentId"])
        .unwrap();
    assert_eq!(
        speed["blocking"], false,
        "warnings fail a row but do not block"
    );
    assert_eq!(speed["author"]["role"], "captions");

    // The same check again keeps the same comments.
    assert!(editor.send(json!({"type": "check", "sequence": "s1"})));
    let again = editor.expect("the second check", |event| {
        event["type"] == "check" && event["status"] == "done"
    });
    assert_eq!(again["sequence"], "s1");
    assert_eq!(
        again["summary"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["code"] == "media")
            .unwrap()["commentId"],
        media["id"]
    );
    let log = RuntimeClient::new(&project.0).unwrap().comments().unwrap();
    assert_eq!(
        log.items
            .iter()
            .filter(|item| item.author.kind == "qc")
            .count(),
        items
            .iter()
            .filter(|item| item["author"]["kind"] == "qc")
            .count()
    );

    // An unknown sequence fails the check, saying so.
    assert!(editor.send(json!({"type": "check", "sequence": "nope"})));
    let failed = editor.expect("the failed check", |event| {
        event["type"] == "check" && event["status"] == "failed"
    });
    assert!(failed["message"].as_str().unwrap().contains("nope"));

    // Solo is monitoring only and is accepted at any time.
    assert!(editor.send(json!({"type": "solo", "tracks": ["A1"]})));
    assert!(editor.send(json!({"type": "solo", "tracks": []})));
}

#[test]
fn the_whole_catalog_lists_generators_and_sound() {
    let pointer = kerf_fx_catalog_all();
    let catalog = take(pointer);
    let entries = catalog.as_array().unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry["kind"] == "generator" && entry["id"] == "title"));
    assert!(entries.iter().any(|entry| entry["domain"] == "audio"));
    assert!(entries
        .iter()
        .any(|entry| entry["kind"] == "effect" && entry["domain"] == "video"));
}

/// A provider that speaks OpenAI's chat API: the first request is answered
/// with a call to Kerf's `edit` tool, every later one with `answer`. Gives
/// its address and the bodies it was sent.
fn stub_provider(answer: &'static str) -> (String, std::sync::mpsc::Receiver<Value>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sent, bodies) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for (turn, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let _ = sent.send(serde_json::from_slice::<Value>(&body).unwrap_or_default());
            let message = if turn == 0 {
                json!({"role": "assistant", "content": null, "tool_calls": [{
                    "id": "call-1", "type": "function",
                    "function": {"name": "edit", "arguments": json!({
                        "script": "marker add at=1s name=\"snare\""
                    }).to_string()},
                }]})
            } else {
                json!({"role": "assistant", "content": answer})
            };
            let finish = if turn == 0 { "tool_calls" } else { "stop" };
            let reply =
                json!({"choices": [{"message": message, "finish_reason": finish}]}).to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    (address, bodies)
}

fn agent(text: &str, base_url: &str, comment: Value) -> Value {
    json!({
        "type": "agent", "text": text, "provider": "openai", "apiMode": "chat",
        "apiKey": "test-key", "baseUrl": base_url, "model": "stub", "comment": comment,
    })
}

#[test]
fn an_agent_turn_is_kept_as_the_persons_comment_and_kerfs_reply() {
    let project = Project::new("turn");
    let mut editor = Editor::open(&project.0);
    editor.expect("kerfd to be ready", |event| {
        event["type"] == "connection" && event["status"] == "ready"
    });
    editor.comments("the first list", |items| items.is_empty());

    let (provider, bodies) = stub_provider("Added a marker on the snare.");
    assert!(editor.send(agent(
        "Mark the snare",
        &provider,
        json!({"at": 26.5, "duration": 0.5}),
    )));
    // The person's request is a comment before Kerf answers.
    let asked = editor.comments("the request", |items| items.len() == 1);
    assert_eq!(asked[0]["author"]["kind"], "person");
    assert_eq!(asked[0]["body"], "Mark the snare");
    assert_eq!(asked[0]["at"], 26.5);
    // The live stream is unchanged.
    editor.expect("the stream's done", |event| {
        event["type"] == "agent" && event["kind"] == "done"
    });
    let items = editor.comments("Kerf's reply", |items| items.len() == 2);
    let reply = &items[1];
    assert_eq!(reply["author"]["kind"], "kerf");
    assert_eq!(reply["parent"], asked[0]["id"]);
    assert_eq!(reply["body"], "Added a marker on the snare.");
    assert_eq!(reply["error"], false);
    let steps = reply["steps"].as_array().unwrap();
    assert!(
        steps
            .iter()
            .any(|step| step.as_str().unwrap().starts_with("edit · done")),
        "{steps:?}"
    );
    // The proposal it staged is linked from the reply and the thread.
    let proposal = reply["proposal"].as_str().expect("the staged proposal");
    assert_eq!(items[0]["proposal"], proposal);
    let state = RuntimeClient::new(&project.0).unwrap().get_state().unwrap();
    assert!(state
        .proposals
        .iter()
        .any(|entry| entry.proposal_id == proposal && entry.status == "pending"));
    assert!(bodies.try_iter().count() >= 2);
    editor.expect("the agent to finish", |event| {
        event["type"] == "agent_busy" && event["busy"] == false
    });

    // Accepting it resolves the thread.
    assert!(
        editor.send(json!({"type": "accept", "proposalId": proposal, "revision": state.revision}))
    );
    let items = editor.comments("the thread resolved", |items| {
        items.first().is_some_and(|root| root["resolved"] == true)
    });
    assert!(items[0]["resolvedRevision"].is_u64());

    // A turn that fails answers in the same thread, as an error.
    assert!(editor.send(agent(
        "Try again",
        "http://127.0.0.1:9/v1",
        json!({"parent": asked[0]["id"]}),
    )));
    let items = editor.comments("the failed reply", |items| {
        items.iter().any(|item| item["error"] == true)
    });
    let failed = items.iter().find(|item| item["error"] == true).unwrap();
    assert_eq!(failed["author"]["kind"], "kerf");
    assert_eq!(failed["parent"], asked[0]["id"]);
    assert!(!failed["body"].as_str().unwrap().is_empty());
    let request = items
        .iter()
        .find(|item| item["body"] == "Try again")
        .unwrap();
    assert_eq!(request["parent"], asked[0]["id"]);
}
