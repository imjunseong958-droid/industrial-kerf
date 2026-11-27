//! A making from the editor, through its bridge and a real kerfd: what its
//! steps say reaches the editor as they say it, a making that is cancelled
//! asks its step to end and leaves no take, and one the editor closes its
//! project on is waited for until its step has ended.
//!
//! Needs kerfd built beside this test (`cargo build -p kerf-runtime --bin
//! kerfd`).
#![cfg(unix)]

use kerf_bridge::Bridge;
use kerf_engine::recipe::model::Models;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
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
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/debug")
        .join(format!("kerfd{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "build kerfd first: cargo build -p kerf-runtime --bin kerfd"
    );
    path
}

/// What the `linger` model writes as the last thing it does, once it has been asked to end.
fn gone() -> PathBuf {
    std::env::temp_dir().join(format!("kq-gone-{}", std::process::id()))
}

/// The models these tests make with, each a program handed the file it writes: one that
/// says two things, the second twice, one that waits, as a model's program waits on
/// its server, and takes its job back when it is asked to end, and one that waits and takes
/// half a second over ending. Written once for this run of the tests, and the only models
/// file read.
fn models() -> &'static Path {
    static FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        let file = std::env::temp_dir().join(format!("kq-models-{}.json", std::process::id()));
        let models = json!({
            "talk": {"run": [
                "sh", "-c",
                "echo blocking >&2; echo asking the model >&2; echo asking the model >&2; printf said > \"$1\"",
                "talk", "{out}",
            ]},
            "wait": {"run": [
                "sh", "-c",
                "trap 'echo took the job back >&2; exit 1' TERM; echo waiting >&2; while :; do sleep 0.05; done",
                "wait", "{out}",
            ]},
            "linger": {"run": [
                "sh", "-c",
                "trap 'sleep 0.5; : > \"$2\"; exit 1' TERM; echo waiting >&2; while :; do sleep 0.05; done",
                "linger", "{out}", gone(),
            ]},
        });
        fs::write(&file, models.to_string()).unwrap();
        file
    })
}

/// A project folder whose media `m1` is made by `steps`, the steps under its line,
/// removed at the end.
struct Project(PathBuf);

impl Project {
    fn new(tag: &str, steps: &[&str]) -> Self {
        // No models a person has set up are read: only these tests' own.
        Models::use_file(models());
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("kq-{tag}-{:x}", stamp % 0xffff_ffff));
        fs::create_dir_all(directory.join("shots")).unwrap();
        let steps: String = steps.iter().map(|step| format!("  {step}\n")).collect();
        fs::write(
            directory.join("project.kerf"),
            format!(
                "kerf 1\nproject name=\"Making\"\n\
                 media m1 path=\"shots/open.mp4\" rate=24 dur=12f video=64x64\n\
                 {steps}\
                 sequence s1 rate=24 size=64x64 audio=48000\n\
                 \x20 track V1 video\n\
                 sequence s2 rate=24 size=64x64 audio=48000\n\
                 \x20 track V1 video\n\
                 \x20   clip g1 gen=solid@1 at=0f dur=12f\n",
            ),
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
    bridge: Bridge,
    events: Vec<Value>,
}

impl Editor {
    fn open(directory: &Path) -> Self {
        let mut editor = Self {
            bridge: Bridge::open(directory, &kerfd()),
            events: Vec::new(),
        };
        editor.expect("the project's state", |event| event["type"] == "state");
        editor
    }

    fn send(&mut self, command: Value) {
        assert!(send(&mut self.bridge, &command), "{command}");
    }

    fn poll(&mut self) -> Option<Value> {
        poll(&mut self.bridge)
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

#[test]
fn a_making_says_what_its_steps_say_as_they_say_it() {
    // A step that says two things, the second twice, and one that has Kerf draw the take.
    let project = Project::new(
        "making",
        &[
            "generate model=talk prompt=\"a word\" out={said.txt}",
            "render s2 {out}",
        ],
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
    let project = Project::new("cancel", &["generate model=wait prompt=\"a word\""]);
    let mut editor = Editor::open(&project.0);
    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true}));
    editor.making("waiting");

    // It is one making of a media at a time.
    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true}));
    let twice = editor.expect("an error", |event| event["type"] == "error");
    assert_eq!(twice["message"], "cannot make m1: it is being made already");
    // Why, without the media its window names already, for the window to say as it is.
    assert_eq!(twice["why"], "it is being made already");
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

#[test]
fn a_bridge_let_go_is_waited_for_until_its_making_has_ended() {
    // A step that waits, and once asked to end takes half a second over it.
    let project = Project::new("linger", &["generate model=linger prompt=\"a word\""]);
    let _ = fs::remove_file(gone());
    let mut editor = Editor::open(&project.0);
    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true}));
    editor.making("waiting");

    // The editor closes the project: the making's program is asked to end.
    drop(editor);
    // What is about to exit waits for it to have ended, and the wait is over no sooner.
    assert!(kerf_bridge::drawing_settled(Duration::from_secs(120)));
    assert!(
        gone().is_file(),
        "the wait was over while the making's program was still ending"
    );
    let _ = fs::remove_file(gone());
}

#[test]
fn several_takes_are_made_one_after_another_and_the_cast_is_read_and_set() {
    let project = Project::new("rolls", &["render s2 {out}"]);
    let mut editor = Editor::open(&project.0);

    // The plan says how many it is for, and nothing runs.
    editor.send(json!({"type": "media_make", "media": "m1", "rolls": 3}));
    let plan = editor.expect("the plan", |event| event["type"] == "media_make_plan");
    assert_eq!((&plan["take"], &plan["rolls"]), (&json!(1), &json!(3)));
    assert!(!project.0.join("shots/open.takes").exists());

    editor.send(json!({"type": "media_make", "media": "m1", "confirmed": true, "rolls": 3}));
    // Each take says that it began, and the media is the last once all are made.
    for take in 1..=3 {
        editor.expect("a take to begin", |event| {
            event["type"] == "media_making" && event["take"] == take && event["line"] == ""
        });
    }
    let made = editor.expect("the takes", |event| event["type"] == "media_made");
    assert_eq!(made["take"], 3);
    editor.expect("the media to be the last take", |event| {
        event["type"] == "state"
            && event["snapshot"]["document"]
                .as_str()
                .is_some_and(|document| document.contains("path=\"shots/open.takes/003.mp4\""))
    });
    for take in ["001", "002", "003"] {
        assert!(project
            .0
            .join(format!("shots/open.takes/{take}.mp4"))
            .is_file());
    }
    assert!(
        !editor.events.iter().any(|event| event["type"] == "error"),
        "{:#?}",
        editor.events
    );

    // A project that says no cast has none to show.
    editor.send(json!({"type": "cast_get"}));
    let none = editor.expect("the cast", |event| event["type"] == "cast");
    assert_eq!(
        (&none["entries"], &none["saved"]),
        (&json!([]), &json!(false))
    );
    // Set, it is the project's cast file, and comes back as it reads.
    editor.send(json!({"type": "cast_set", "entries": [
        {"kind": "cast", "name": "Ada", "words": "an  old keeper", "refs": ["refs/ada.png"]},
        {"kind": "style", "name": "film", "words": "35mm"},
    ]}));
    let saved = editor.expect("the cast", |event| event["type"] == "cast");
    assert_eq!(saved["saved"], true);
    assert_eq!(
        saved["entries"],
        json!([
            {"kind": "cast", "name": "Ada", "words": "an old keeper", "refs": ["refs/ada.png"]},
            {"kind": "style", "name": "film", "words": "35mm", "refs": []},
        ])
    );
    assert_eq!(
        fs::read_to_string(project.0.join("cast.kerfcast")).unwrap(),
        "kerf-cast 1\ncast Ada \"an old keeper\" ref=refs/ada.png\nstyle film \"35mm\"\n"
    );
    // One that does not read is an error that says so, and the file is as it was.
    editor.send(json!({"type": "cast_set", "entries": [{"kind": "cast", "name": "the keeper", "words": "old"}]}));
    let refused = editor.expect("an error", |event| event["type"] == "error");
    assert_eq!(refused["command"], "cast");
    assert!(refused["message"]
        .as_str()
        .is_some_and(|message| message.starts_with("cannot save the cast:")));
    assert!(fs::read_to_string(project.0.join("cast.kerfcast"))
        .unwrap()
        .contains("cast Ada"));
}

#[test]
fn a_recipe_file_is_carried_into_the_document_once_a_person_says_yes() {
    let project = Project::new("carry", &[]);
    let file = project.0.join("project.kerf");
    let text = fs::read_to_string(&file).unwrap().replace(
        "video=64x64\n",
        "video=64x64 recipe=\"shots/open.kerfrecipe\"\n",
    );
    fs::write(&file, text).unwrap();
    fs::write(
        project.0.join("shots/open.kerfrecipe"),
        "kerf-recipe 1\nrender s2 {out}\n",
    )
    .unwrap();
    let mut editor = Editor::open(&project.0);

    // Nothing is made from the recipe file: how to carry its steps is said.
    editor.send(json!({"type": "media_make", "media": "m1"}));
    let refused = editor.expect("an error", |event| event["type"] == "error");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("`kerf carry m1`")),
        "{refused}"
    );
    // And which refusal it is, by its code: what the editor offers to carry the steps by.
    assert_eq!(refused["code"], "STEPS_NOT_CARRIED", "{refused}");
    // How it is made is refused the same way, and by the same code.
    editor.send(json!({"type": "recipe_get", "media": "m1"}));
    let unread = editor.expect("an error", |event| {
        event["type"] == "error" && event["command"] == "recipe"
    });
    assert_eq!(unread["code"], "STEPS_NOT_CARRIED", "{unread}");
    // So is a change asked of its steps, in the first thing said of it.
    editor.send(json!({"type": "recipe_edit", "media": "m1", "was": "", "line": 1, "key": "prompt", "value": "x"}));
    let unchanged = editor.expect("an error", |event| {
        event["type"] == "error" && event["command"] == "recipe"
    });
    assert_eq!(unchanged["code"], "STEPS_NOT_CARRIED", "{unchanged}");
    // A refusal that is not that one does not say it is.
    editor.send(json!({"type": "recipe_get", "media": "nowhere"}));
    let missing = editor.expect("an error", |event| {
        event["type"] == "error" && event["media"] == "nowhere"
    });
    assert!(missing.get("code").is_none(), "{missing}");

    // Asked for plainly, what would be written is shown and nothing is.
    editor.send(json!({"type": "media_carry", "media": "m1"}));
    let plan = editor.expect("the plan", |event| event["type"] == "media_carry_plan");
    assert_eq!(
        (&plan["media"], &plan["recipe"]),
        (&json!("m1"), &json!("shots/open.kerfrecipe"))
    );
    assert_eq!(
        plan["script"].as_array().unwrap().last().unwrap(),
        "step m1 add render s2 {out}"
    );
    assert!(fs::read_to_string(&file).unwrap().contains("recipe="));

    // Confirmed, the steps are under the media's line, one edit, and the file is left.
    editor.send(json!({"type": "media_carry", "media": "m1", "confirmed": true}));
    let carried = editor.expect("the carry", |event| event["type"] == "media_carried");
    assert_eq!(
        carried,
        json!({"type": "media_carried", "media": "m1", "recipe": "shots/open.kerfrecipe", "held": 0})
    );
    editor.expect("the steps in the document", |event| {
        event["type"] == "state"
            && event["snapshot"]["document"]
                .as_str()
                .is_some_and(|document| {
                    document.contains("\n  render s2 {out}\n") && !document.contains("recipe=")
                })
    });
    assert!(project.0.join("shots/open.kerfrecipe").is_file());

    // Carried, there is nothing to carry again: that is said, naming the media.
    editor.send(json!({"type": "media_carry", "media": "m1"}));
    let again = editor.expect("an error", |event| {
        event["type"] == "error" && event["command"] == "media_carry"
    });
    assert_eq!(again["media"], "m1");
    assert!(
        again["message"]
            .as_str()
            .is_some_and(|message| message.contains("has its steps under its line")),
        "{again}"
    );
    assert_eq!(
        again["message"],
        format!(
            "cannot carry m1: {}",
            again["why"].as_str().unwrap_or_default()
        ),
        "{again}"
    );

    // A change of the carried steps that the runtime refuses is said by the runtime's code:
    // here a prompt too long for one request to kerfd.
    let steps = "kerf-recipe 1\nrender s2 {out}\n";
    let prompt = "a".repeat(1100 * 1024);
    editor.send(json!({"type": "recipe_edit", "media": "m1", "was": steps,
        "text": format!("kerf-recipe 1\ngenerate model=talk prompt=\"{prompt}\" out={{said.txt}}\nrender s2 {{out}}\n")}));
    let refused = editor.expect("the runtime's refusal", |event| {
        event["type"] == "error"
            && event["command"] == "recipe"
            && event["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("request exceeds"))
    });
    assert_eq!(refused["code"], "REQUEST_TOO_LARGE", "{refused}");
}
