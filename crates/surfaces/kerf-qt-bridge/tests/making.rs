//! A making from the editor, through its C ABI and a real kerfd: what its
//! steps say reaches the editor as they say it, and a making that is
//! cancelled asks its step to end and leaves no take.
//!
//! Needs kerfd built beside this test (`cargo build -p kerf-runtime --bin
//! kerfd`).
#![cfg(unix)]

use kerf_qt_bridge::{
    kerf_bridge_free, kerf_bridge_new, kerf_bridge_poll, kerf_bridge_send, kerf_bridge_string_free,
};
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

/// A project folder whose media `m1` is made by `recipe`, removed at the end.
struct Project(PathBuf);

impl Project {
    fn new(tag: &str, recipe: &str) -> Self {
        // No models a person has set up are read.
        std::env::set_var("KERF_MODELS", "/no-such-kerf-models.json");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("kq-{tag}-{:x}", stamp % 0xffff_ffff));
        fs::create_dir_all(directory.join("shots")).unwrap();
        fs::write(
            directory.join("shots/open.kerfrecipe"),
            format!("kerf-recipe 1\n{recipe}\n"),
        )
        .unwrap();
        fs::write(
            directory.join("project.kerf"),
            "kerf 1\nproject name=\"Making\"\n\
             media m1 path=\"shots/open.mp4\" rate=24 dur=12f video=64x64 recipe=\"shots/open.kerfrecipe\"\n\
             sequence s1 rate=24 size=64x64 audio=48000\n\
             \x20 track V1 video\n\
             sequence s2 rate=24 size=64x64 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=12f\n",
        )
        .unwrap();
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
        let mut editor = Self {
            bridge,
            events: Vec::new(),
        };
        editor.expect("the project's state", |event| event["type"] == "state");
        editor
    }

    fn send(&self, command: Value) {
        let text = CString::new(command.to_string()).unwrap();
        // SAFETY: the bridge is live and only this thread uses it.
        assert!(
            unsafe { kerf_bridge_send(self.bridge, text.as_ptr()) },
            "{command}"
        );
    }

    fn poll(&mut self) -> Option<Value> {
        // SAFETY: the bridge is live and only this thread uses it.
        let pointer = unsafe { kerf_bridge_poll(self.bridge) };
        (!pointer.is_null()).then(|| take(pointer))
    }

    /// The next event that `matches`, among those not yet taken.
    fn expect(&mut self, what: &str, matches: impl Fn(&Value) -> bool) -> Value {
        if let Some(index) = self.events.iter().position(&matches) {
            return self.events.remove(index);
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            match self.poll() {
                None => std::thread::sleep(Duration::from_millis(5)),
                Some(event) if matches(&event) => return event,
                Some(event) => self.events.push(event),
            }
        }
        let recent = &self.events[self.events.len().saturating_sub(12)..];
        panic!("no event for {what}; recent events: {recent:#?}");
    }

    /// The line of the next `media_making` event that says `line`.
    fn making(&mut self, line: &str) -> Value {
        self.expect(&format!("the making to say `{line}`"), |event| {
            event["type"] == "media_making" && event["line"] == line
        })
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
fn a_making_says_what_its_steps_say_as_they_say_it() {
    // A step that says two things, the second twice, and one that has Kerf draw the take.
    let project = Project::new(
        "making",
        "run sh -c \"echo blocking >&2; echo asking the model >&2; echo asking the model >&2\"\nrender s2 {out}",
    );
    let mut editor = Editor::open(&project.0);

    // Asked for plainly, the steps are shown and nothing runs or is said to be made.
    editor.send(json!({"type": "media_make", "media": "m1"}));
    let plan = editor.expect("the plan", |event| event["type"] == "media_make_plan");
    assert_eq!(plan["take"], 1);
    assert!(!project.0.join("shots/open.takes").exists());

    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true}));
    // It says that it began, with the take it is making, before any step speaks.
    let began = editor.making("");
    assert_eq!((&began["media"], &began["take"]), (&json!("m1"), &json!(1)));
    editor.making("blocking");
    editor.making("asking the model");
    // What Kerf draws is said as it goes too.
    editor.making("rendering s2: 100%");
    let made = editor.expect("the take", |event| event["type"] == "media_made");
    assert_eq!((&made["media"], &made["take"]), (&json!("m1"), &json!(1)));
    // A line said again while nothing changed was said once.
    assert!(
        !editor
            .events
            .iter()
            .any(|event| event["type"] == "media_making" && event["line"] == "asking the model"),
        "{:#?}",
        editor.events
    );
    editor.expect("the media to be the take", |event| {
        event["type"] == "state"
            && event["snapshot"]["document"]
                .as_str()
                .is_some_and(|document| document.contains("path=\"shots/open.takes/001.mp4\""))
    });
    assert!(project.0.join("shots/open.takes/001.mp4").is_file());
    assert!(
        !editor.events.iter().any(|event| event["type"] == "error"),
        "{:#?}",
        editor.events
    );

    // Nothing is being made now, so there is nothing to stop.
    editor.send(json!({"type": "media_make_cancel", "media": "m1"}));
    let none = editor.expect("an error", |event| event["type"] == "error");
    assert_eq!(none["message"], "m1 is not being made");
}

#[test]
fn a_making_that_is_cancelled_asks_its_step_to_end_and_leaves_no_take() {
    // A step that waits, as a model's program waits on its server, and takes its job back
    // when it is asked to end.
    let project = Project::new(
        "cancel",
        r#"run sh -c "trap 'echo took the job back >&2; exit 1' TERM; echo waiting >&2; while :; do sleep 0.05; done" step {out}"#,
    );
    let mut editor = Editor::open(&project.0);
    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true}));
    editor.making("waiting");

    // It is one making of a media at a time.
    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true}));
    let twice = editor.expect("an error", |event| event["type"] == "error");
    assert_eq!(twice["message"], "cannot make m1: it is being made already");
    assert_eq!(
        (&twice["command"], &twice["media"]),
        (&json!("media_make"), &json!("m1"))
    );

    let asked = Instant::now();
    editor.send(json!({"type": "media_make_cancel", "media": "m1"}));
    // What the step says on its way out is still heard.
    editor.making("took the job back");
    let cancelled = editor.expect("the making to stop", |event| {
        event["type"] == "media_make_cancelled"
    });
    assert_eq!(cancelled["media"], "m1");
    assert!(
        asked.elapsed() < Duration::from_secs(4),
        "{:?}",
        asked.elapsed()
    );
    // It is not an error, and no take or part of one is left.
    assert!(
        !editor.events.iter().any(|event| event["type"] == "error"),
        "{:#?}",
        editor.events
    );
    let left: Vec<String> = fs::read_dir(project.0.join("shots/open.takes"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");

    // It can be made again afterwards.
    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true}));
    editor.making("waiting");
    editor.send(json!({"type": "media_make_cancel", "media": "m1"}));
    editor.expect("the making to stop", |event| {
        event["type"] == "media_make_cancelled"
    });
}
