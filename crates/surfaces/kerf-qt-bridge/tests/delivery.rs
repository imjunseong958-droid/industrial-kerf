//! Delivery through the editor's C ABI and a real kerfd: the render queue
//! with a ProRes master, the paperwork beside it, programme loudness, what
//! the media say of themselves, their index, and effects presets.
//!
//! Needs kerfd built beside this test (`cargo build -p kerf-runtime --bin
//! kerfd`) and FFmpeg; without FFmpeg it says so and passes.

use kerf_qt_bridge::{
    kerf_bridge_free, kerf_bridge_new, kerf_bridge_poll, kerf_bridge_send, kerf_bridge_string_free,
};
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

fn ffmpeg() -> String {
    std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

/// A project folder, removed at the end, which ends its daemon.
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
        let deadline = Instant::now() + Duration::from_secs(90);
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
        let recent = &self.events[self.events.len().saturating_sub(12)..];
        panic!("no event for {what}; recent events: {recent:#?}");
    }

    /// The newest state snapshot, once one matches.
    fn state(&mut self, what: &str, matches: impl Fn(&Value) -> bool) -> Value {
        self.expect(what, |event| {
            event["type"] == "state" && matches(&event["snapshot"])
        })["snapshot"]
            .clone()
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

/// Two seconds of picture that cuts from dark grey to light grey at 1 s,
/// under a tone that starts after half a second of silence.
fn media(path: &Path) -> bool {
    Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=0x202020:s=160x90:r=24:d=1"])
        .args(["-f", "lavfi", "-i", "color=0xd0d0d0:s=160x90:r=24:d=1"])
        .args(["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo:d=0.5"])
        .args(["-f", "lavfi", "-i", "sine=f=440:r=48000:d=1.5"])
        .args([
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1:a=0[v];[2:a][3:a]concat=n=2:v=0:a=1,aformat=channel_layouts=stereo[a]",
            "-map",
            "[v]",
            "-map",
            "[a]",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ])
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

fn markers(snapshot: &Value) -> Vec<Value> {
    let document = snapshot["document"].as_str().unwrap_or_default();
    document
        .lines()
        .filter(|line| line.trim_start().starts_with("marker "))
        .map(|line| json!(line.trim()))
        .collect()
}

#[test]
fn a_master_renders_in_the_queue_and_its_paperwork_follows() {
    let project = Project::new("deliver");
    fs::create_dir_all(project.0.join("media")).unwrap();
    if !media(&project.0.join("media/tone.mp4")) {
        eprintln!("skipping: FFmpeg could not make the test media");
        return;
    }
    fs::write(
        project.0.join("project.kerf"),
        "kerf 1\nproject name=\"Deliver\"\n\
         media m1 path=\"media/tone.mp4\" rate=24 dur=48f video=160x90 audio=2ch@48000\n\
         sequence s1 rate=24 size=160x90 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=0f at=0f dur=24f\n\
         \x20     fx color.basic@1 exposure=0.3\n\
         \x20   clip c2 media=m1 src=24f at=24f dur=24f\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=0f at=0f dur=48f\n\
         \x20 track S1 subtitle\n\
         \x20   clip t1 caption=\"Hello there\" at=0f dur=24f\n",
    )
    .unwrap();
    let mut editor = Editor::open(&project.0);
    editor.expect("kerfd to be ready", |event| {
        event["type"] == "connection" && event["status"] == "ready"
    });

    // What the media say of themselves comes on its own once opened.
    let info = editor.expect("media info", |event| event["type"] == "media_info");
    assert_eq!(info["media"]["m1"]["codec"], "h264", "{info}");
    assert_eq!(info["media"]["m1"]["pixFmt"], "yuv420p", "{info}");
    assert!(editor.send(json!({"type": "media_info", "media": "m1"})));
    let asked = editor.expect("media info asked for", |event| {
        event["type"] == "media_info"
    });
    assert_eq!(asked["media"]["m1"]["bitDepth"], 8, "{asked}");

    // The index: one cut at 1 s, so two shots, and the opening silence.
    assert!(editor.send(json!({"type": "media_index", "media": "m1"})));
    let index = editor.expect("the index", |event| {
        event["type"] == "media_index" || event["command"] == "media_index"
    });
    assert_eq!(index["type"], "media_index", "{index}");
    assert_eq!(index["ranges"].as_array().unwrap().len(), 2, "{index}");
    assert!(index["silences"][0][1].as_f64().unwrap() > 0.3, "{index}");

    // A person's note at 0.5 s.
    assert!(editor.send(json!({"type": "comment_add", "body": "Hold on the grey", "at": 0.5})));
    editor.expect("the note", |event| {
        event["type"] == "comments"
            && event["items"]
                .as_array()
                .is_some_and(|items| items.len() == 1)
    });

    // Three exports queue at once and render oldest first; the second is
    // cancelled before its turn or while it renders.
    assert!(editor.send(json!({"type": "export", "quality": "full", "name": "First"})));
    assert!(editor.send(json!({"type": "export", "quality": "full", "name": "Second"})));
    assert!(editor.send(json!({"type": "export", "quality": "master", "name": "Master"})));
    let first = editor.expect("the first queued", |event| {
        event["type"] == "export" && event["status"] == "queued" && event["name"] == "First"
    });
    let second = editor.expect("the second queued", |event| {
        event["type"] == "export" && event["status"] == "queued" && event["name"] == "Second"
    });
    let master = editor.expect("the master queued", |event| {
        event["type"] == "export" && event["status"] == "queued" && event["name"] == "Master"
    });
    assert_eq!(master["codec"], "prores_hq");
    assert!(
        master["path"].as_str().unwrap().ends_with("Master.mov"),
        "{master}"
    );
    assert!(editor.send(json!({"type": "export_cancel", "jobId": second["jobId"]})));
    editor.expect("the second cancelled", |event| {
        event["type"] == "export"
            && event["status"] == "cancelled"
            && event["jobId"] == second["jobId"]
    });
    let done = editor.expect("the first rendered", |event| {
        event["type"] == "export"
            && event["jobId"] == first["jobId"]
            && event["status"] != "running"
    });
    assert_eq!(done["status"], "completed", "{done}");
    assert!(Path::new(done["path"].as_str().unwrap()).is_file());
    let mastered = editor.expect("the master rendered", |event| {
        event["type"] == "export"
            && event["jobId"] == master["jobId"]
            && event["status"] != "running"
            && event["status"] != "queued"
    });
    assert_eq!(mastered["status"], "completed", "{mastered}");
    let master_file = PathBuf::from(mastered["path"].as_str().unwrap());
    assert!(master_file.is_file());
    assert!(editor.send(json!({"type": "render_queue"})));
    let queue = editor.expect("the queue", |event| {
        event["type"] == "render_queue"
            && event["jobs"].as_array().is_some_and(|jobs| {
                jobs.iter()
                    .any(|job| job["jobId"] == master["jobId"] && job["status"] == "completed")
            })
    });
    let statuses: Vec<(String, String)> = queue["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| {
            (
                job["name"].as_str().unwrap().to_owned(),
                job["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [
            ("First".to_owned(), "completed".to_owned()),
            ("Second".to_owned(), "cancelled".to_owned()),
            ("Master".to_owned(), "completed".to_owned())
        ]
    );

    // The paperwork goes beside the master.
    let folder = master_file.parent().unwrap().to_path_buf();
    assert!(
        editor.send(json!({"type": "export_comments", "format": "csv", "includeResolved": false}))
    );
    let csv = editor.expect("the sheet", |event| {
        event["type"] == "export" && event["kind"] == "paperwork" && event["format"] == "csv"
    });
    assert_eq!(csv["status"], "completed", "{csv}");
    assert_eq!(
        PathBuf::from(csv["path"].as_str().unwrap()),
        folder.join("Master comments.csv")
    );
    let sheet = fs::read_to_string(folder.join("Master comments.csv")).unwrap();
    assert!(
        sheet.contains("00:00:00:12") && sheet.contains("Hold on the grey"),
        "{sheet}"
    );
    assert!(editor.send(json!({"type": "export_comments", "format": "edl"})));
    let edl = editor.expect("the EDL", |event| {
        event["type"] == "export" && event["kind"] == "paperwork" && event["format"] == "edl"
    });
    assert_eq!(edl["status"], "completed", "{edl}");
    let text = fs::read_to_string(folder.join("Master.edl")).unwrap();
    assert!(
        text.contains("* LOC: 00:00:00:12 YELLOW") && text.contains("Hold on the grey"),
        "{text}"
    );
    assert!(editor.send(json!({"type": "export_captions", "format": "srt"})));
    let srt = editor.expect("the captions", |event| {
        event["type"] == "export" && event["kind"] == "captions"
    });
    assert_eq!(srt["status"], "completed", "{srt}");
    assert!(fs::read_to_string(folder.join("Master.srt"))
        .unwrap()
        .contains("Hello there"));

    // Markers: one edit, and a second export sets the same marker again.
    for _ in 0..2 {
        assert!(editor.send(json!({"type": "export_comments", "format": "markers"})));
        let marked = editor.expect("the markers", |event| {
            event["type"] == "export" && event["format"] == "markers"
        });
        assert_eq!(marked["status"], "completed", "{marked}");
        assert_eq!(marked["count"], 1);
    }
    let snapshot = editor.state("the marker", |snapshot| !markers(snapshot).is_empty());
    assert_eq!(markers(&snapshot).len(), 1, "{:?}", markers(&snapshot));
    assert!(markers(&snapshot)[0].as_str().unwrap().contains("note-"));

    // Loudness, then the same revision again at once.
    assert!(editor.send(json!({"type": "loudness"})));
    editor.expect("loudness running", |event| {
        event["type"] == "loudness" && event["status"] == "running"
    });
    let measured = editor.expect("loudness", |event| {
        event["type"] == "loudness" && event["status"] != "running"
    });
    assert_eq!(measured["status"], "ready", "{measured}");
    let integrated = measured["integrated"]
        .as_f64()
        .expect("a tone has a loudness");
    assert!(integrated < -3.0 && integrated > -40.0, "{measured}");
    assert_eq!(measured["target"], -23.0);
    assert!(!measured["series"].as_array().unwrap().is_empty());
    assert!(editor.send(json!({"type": "loudness"})));
    let kept = editor.expect("loudness again", |event| {
        event["type"] == "loudness" && event["status"] != "running"
    });
    assert_eq!(kept["revision"], measured["revision"]);
    assert_eq!(kept["integrated"], measured["integrated"]);

    // An effects preset from c1, put on c2.
    assert!(editor.send(json!({"type": "look_save", "clip": "c1", "name": "Harbor warm"})));
    let saved = editor.expect("the saved preset", |event| {
        event["type"] == "look_saved" || event["command"] == "look_save"
    });
    assert_eq!(
        saved["path"], "presets/looks/Harbor warm.kerflook",
        "{saved}"
    );
    assert!(project
        .0
        .join("presets/looks/Harbor warm.kerflook")
        .is_file());
    assert!(editor.send(json!({"type": "look_apply", "clip": "c2", "path": saved["path"]})));
    let applied = editor.expect("the applied preset", |event| {
        event["type"] == "look_applied" || event["command"] == "look_apply"
    });
    assert_eq!(applied["fx"], "look.harbor_warm", "{applied}");
    let snapshot = editor.state("c2 with the look", |snapshot| {
        snapshot["document"]
            .as_str()
            .is_some_and(|document| document.contains("fx look.harbor_warm@1"))
    });
    assert!(snapshot["document"]
        .as_str()
        .unwrap()
        .contains("look.harbor_warm"));
    assert!(project
        .0
        .join(".kerf/looks/look.harbor_warm@1.kerflook")
        .is_file());
}
