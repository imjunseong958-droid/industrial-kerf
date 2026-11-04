//! What the other suites leave out, through the editor's C ABI and a real
//! kerfd: the meter while a -23 dBFS tone plays, solo kept across a kerfd
//! restart, keyframes kept through it, proposals waiting side by side and a
//! revised version, a proposal previewed in place of the program, and
//! titles placed by the generator insert op on a sequence whose frame rate
//! was set while it was empty.
//!
//! Needs kerfd built beside this test (`cargo build -p kerf-runtime --bin
//! kerfd`) and FFmpeg; the tests that need media say so and pass without
//! FFmpeg.

use kerf_qt_bridge::{
    kerf_bridge_frame, kerf_bridge_free, kerf_bridge_new, kerf_bridge_poll, kerf_bridge_send,
    kerf_bridge_string_free, KerfFrame,
};
use kerf_runtime::{Author, RuntimeClient};
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn kerfd() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
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

/// kerfd for a project, started here so the test can stop it.
struct Daemon(Child);

impl Daemon {
    fn start(directory: &Path) -> Self {
        let child = Command::new(kerfd())
            .arg("--dir")
            .arg(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn kerfd");
        let daemon = Self(child);
        let client = RuntimeClient::new(directory).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while client.get_state().is_err() {
            assert!(Instant::now() < deadline, "kerfd did not open its socket");
            std::thread::sleep(Duration::from_millis(20));
        }
        daemon
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
        let deadline = Instant::now() + Duration::from_secs(120);
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

    /// The next state snapshot that `matches`.
    fn state(&mut self, what: &str, matches: impl Fn(&Value) -> bool) -> Value {
        self.expect(what, |event| {
            event["type"] == "state" && matches(&event["snapshot"])
        })["snapshot"]
            .clone()
    }

    /// Every event that matches over the next `seconds`.
    fn gather(&mut self, seconds: f64, matches: impl Fn(&Value) -> bool) -> Vec<Value> {
        let (mut taken, kept) = std::mem::take(&mut self.events)
            .into_iter()
            .partition(&matches);
        self.events = kept;
        let deadline = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < deadline {
            match self.poll() {
                None => std::thread::sleep(Duration::from_millis(5)),
                Some(event) if matches(&event) => taken.push(event),
                Some(event) => self.events.push(event),
            }
        }
        taken
    }

    fn forget(&mut self, kind: &str) {
        self.events.retain(|event| event["type"] != kind);
    }

    /// The program's RGBA frame at `position`, and its size.
    fn frame_at(&mut self, position: f64) -> (Vec<u8>, u32, u32) {
        self.send(json!({"type": "seek", "position": position}));
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Some(event) = self.poll() {
                assert_ne!(event["type"], "error", "{event}");
                self.events.push(event);
            }
            let mut frame = KerfFrame {
                pixels: std::ptr::null(),
                length: 0,
                width: 0,
                height: 0,
                time: 0.0,
            };
            // SAFETY: the bridge is live and the frame is writable.
            if unsafe { kerf_bridge_frame(self.bridge, &mut frame) }
                && (frame.time - position).abs() < 0.05
            {
                // SAFETY: the pixels live until the next frame poll.
                let pixels = unsafe { std::slice::from_raw_parts(frame.pixels, frame.length) };
                return (pixels.to_vec(), frame.width, frame.height);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no frame at {position}");
    }

    /// The meter's readings and the clocks while the program plays for
    /// `seconds` from half a second in.
    fn play(&mut self, seconds: f64) -> (Vec<Value>, Vec<Value>) {
        self.forget("meter");
        self.forget("clock");
        self.send(json!({"type": "seek", "position": 0.5}));
        self.send(json!({"type": "play"}));
        // A busy machine may take a moment to start the sound.
        let first = self.expect("the first reading", |event| {
            event["type"] == "meter" && !event["peak"].is_null()
        });
        let mut events = vec![first];
        events.extend(self.gather(seconds, |event| {
            event["type"] == "meter" || event["type"] == "clock"
        }));
        self.send(json!({"type": "pause"}));
        self.expect("the meter cleared on pause", |event| {
            event["type"] == "meter" && event["peak"].is_null()
        });
        let (readings, clocks): (Vec<Value>, Vec<Value>) = events
            .into_iter()
            .filter(|event| event["type"] == "clock" || !event["peak"].is_null())
            .partition(|event| event["type"] == "meter");
        assert!(readings.len() >= 10, "{readings:?}");
        (readings, clocks)
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

/// Four seconds of `picture` at 320x180 and 24 fps over stereo `sound`.
fn media(path: &Path, picture: &str, sound: &str) -> bool {
    Command::new(ffmpeg())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("{picture}=size=320x180:rate=24:duration=4"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("{sound}:duration=4"))
        .args(["-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-preset"])
        .args([
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
        ])
        .arg("-shortest")
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

/// A 1 kHz tone at -23 dBFS in both channels (so -23 LUFS) on A1, under a
/// silent A2.
const TONE_PROJECT: &str = "kerf 1\nproject name=\"Tone\"\n\
    media m1 path=\"media/tone.mp4\" rate=24 dur=96f video=320x180 audio=2ch@48000\n\
    media m2 path=\"media/quiet.mp4\" rate=24 dur=96f video=320x180 audio=2ch@48000\n\
    sequence s1 rate=24 size=320x180 audio=48000\n\
    \x20 track V1 video\n\
    \x20   clip c1 media=m1 src=0f at=0f dur=96f\n\
    \x20 track A1 audio\n\
    \x20   clip a1 media=m1 src=0f at=0f dur=96f\n\
    \x20 track A2 audio\n\
    \x20   clip a2 media=m2 src=0f at=0f dur=96f\n";

/// The tone project, or `None` when FFmpeg cannot make its media.
fn tone_project(tag: &str) -> Option<Project> {
    let project = Project::new(tag);
    fs::create_dir_all(project.0.join("media")).unwrap();
    let tone = "aevalsrc=0.0707946*sin(2*PI*1000*t)|0.0707946*sin(2*PI*1000*t):c=stereo:s=48000";
    if !media(&project.0.join("media/tone.mp4"), "testsrc2", tone)
        || !media(
            &project.0.join("media/quiet.mp4"),
            "smptebars",
            "anullsrc=r=48000:cl=stereo",
        )
    {
        eprintln!("skipping: FFmpeg could not make the test media");
        return None;
    }
    fs::write(project.0.join("project.kerf"), TONE_PROJECT).unwrap();
    Some(project)
}

fn ready(editor: &mut Editor) {
    editor.expect("kerfd to be ready", |event| {
        event["type"] == "connection" && event["status"] == "ready"
    });
}

fn clip<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|track| track["clips"].as_array().into_iter().flatten())
        .find(|clip| clip["id"] == id)
        .unwrap_or_else(|| panic!("no clip {id}: {}", snapshot["document"]))
}

fn loudest(reading: &Value, field: &str) -> f64 {
    reading[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .fold(f64::NEG_INFINITY, f64::max)
}

fn heard(readings: &[Value]) -> Vec<f64> {
    readings[readings.len() / 2..]
        .iter()
        .map(|reading| loudest(reading, "peak"))
        .collect()
}

#[test]
fn the_meter_reads_a_tone_and_solo_holds_across_a_kerfd_restart() {
    let Some(project) = tone_project("meter") else {
        return;
    };
    let mut daemon = Daemon::start(&project.0);
    let mut editor = Editor::open(&project.0);
    ready(&mut editor);
    let opened = editor.state("the project", |snapshot| {
        snapshot["document"].as_str().unwrap_or("").contains("a2")
    });
    // Volume applies after the meter: the speakers stay quiet.
    editor.send(json!({"type": "volume", "value": 0.0}));

    // The tone as played: -23 dBFS peaks, -26 dBFS RMS, -23 LUFS.
    let (readings, clocks) = editor.play(2.0);
    let settled = &readings[readings.len() / 2..];
    for reading in settled {
        let peak = loudest(reading, "peak");
        let rms = loudest(reading, "rms");
        assert!((-24.5..-21.5).contains(&peak), "{reading}");
        assert!((-27.5..-24.5).contains(&rms), "{reading}");
        if let Some(momentary) = reading["momentary"].as_f64() {
            assert!((momentary + 23.0).abs() < 1.5, "{reading}");
        }
    }
    assert!(settled.iter().any(|reading| reading["momentary"].is_f64()));
    assert!(readings.last().unwrap()["at"].as_f64() > readings[0]["at"].as_f64());
    assert!(!clocks.is_empty());
    assert!(
        clocks.iter().all(|clock| clock["dropped"].is_u64()),
        "{clocks:?}"
    );

    // Keys as the Inspector writes them: frames from the clip's start.
    editor.send(
        json!({"type": "edit", "revision": opened["revision"], "ops": [{
        "op": "set", "clip": "c1", "props": {"opacity": {"keys": [
            {"at": 0, "value": 1.0, "interp": "linear"},
            {"at": 24, "value": 0.25, "interp": "ease"}]}}}]}),
    );
    let keyed = editor.state("the keys", |snapshot| {
        snapshot["revision"].as_u64() > opened["revision"].as_u64()
    });
    let keys = json!([
        {"at": 0.0, "frame": 0, "interp": "linear", "value": 1.0},
        {"at": 1.0, "frame": 24, "interp": "ease", "value": 0.25}
    ]);
    assert_eq!(clip(&keyed, "c1")["props"]["opacity"], keys);

    // Solo the silent track: nothing is heard.
    editor.send(json!({"type": "solo", "tracks": ["A2"]}));
    let (readings, _) = editor.play(1.5);
    assert!(
        heard(&readings).iter().all(|peak| *peak < -90.0),
        "{readings:?}"
    );

    // kerfd goes away and comes back: the bridge keeps the solo, and the
    // keys come back from the project kerfd wrote.
    drop(daemon);
    editor.expect("kerfd gone", |event| {
        event["type"] == "connection" && event["status"] == "disconnected"
    });
    daemon = Daemon::start(&project.0);
    editor.forget("state");
    editor.send(json!({"type": "reconnect"}));
    ready(&mut editor);
    let reopened = editor.state("the project again", |snapshot| {
        snapshot["document"].as_str().unwrap_or("").contains("a2")
    });
    assert_eq!(clip(&reopened, "c1")["props"]["opacity"], keys);
    assert!(fs::read_to_string(project.0.join("project.kerf"))
        .unwrap()
        .contains("set opacity={0f:1 24f:0.25:ease}"));
    let (readings, _) = editor.play(1.5);
    assert!(
        heard(&readings).iter().all(|peak| *peak < -90.0),
        "{readings:?}"
    );
    // The editor sends the solo again on reconnecting; hearing all again.
    editor.send(json!({"type": "solo", "tracks": ["A2"]}));
    let (readings, _) = editor.play(1.5);
    assert!(
        heard(&readings).iter().all(|peak| *peak < -90.0),
        "{readings:?}"
    );
    editor.send(json!({"type": "solo", "tracks": []}));
    let (readings, _) = editor.play(1.5);
    assert!(
        heard(&readings).iter().all(|peak| *peak > -25.0),
        "{readings:?}"
    );
    drop(editor);
    drop(daemon);
}

fn proposal<'a>(snapshot: &'a Value, id: &str) -> Option<&'a Value> {
    snapshot["proposals"]
        .as_array()?
        .iter()
        .find(|entry| entry["proposalId"] == id)
}

fn pending(snapshot: &Value) -> usize {
    snapshot["proposals"].as_array().map_or(0, |entries| {
        entries
            .iter()
            .filter(|entry| entry["status"] == "pending")
            .count()
    })
}

#[test]
fn proposals_wait_side_by_side_and_one_previews_in_place_of_the_program() {
    let Some(project) = tone_project("proposals") else {
        return;
    };
    let mut editor = Editor::open(&project.0);
    ready(&mut editor);
    let client = RuntimeClient::new(&project.0).unwrap();
    let base = client.get_state().unwrap().revision;
    let agent = Author {
        id: Some("codex".into()),
        ..Author::new("mcp", "Codex")
    };
    let kerf = client
        .stage_script_proposal_by(
            "kerf-1",
            base,
            "marker add at=1s name=\"one\"",
            None,
            &Author::kerf(),
            None,
        )
        .unwrap();
    let kerf_id = kerf.proposals[0].proposal_id.clone();
    let staged = client
        .stage_script_proposal_by(
            "agent-1",
            base,
            "marker add at=2s name=\"two\"",
            None,
            &agent,
            None,
        )
        .unwrap();
    let agent_id = staged.proposals[0].proposal_id.clone();
    assert_ne!(kerf_id, agent_id);
    let both = editor.state("two pending", |snapshot| pending(snapshot) == 2);
    assert_eq!(
        proposal(&both, &agent_id).unwrap()["author"]["name"],
        "Codex"
    );
    assert!(both["proposalTimelines"][&kerf_id].is_object());
    assert!(both["proposalTimelines"][&agent_id].is_object());

    // A note about Kerf's proposal, then accepting it: the note is
    // resolved at the revision the proposal made, and the other is stale.
    editor.send(json!({"type": "comment_add", "body": "Take this one", "proposal": kerf_id}));
    editor.expect("the note", |event| {
        event["type"] == "comments" && event["comment"]["proposal"] == kerf_id.as_str()
    });
    editor.send(json!({"type": "accept", "proposalId": kerf_id, "revision": both["revision"]}));
    let accepted = editor.state("the accept", |snapshot| {
        snapshot["revision"].as_u64() > both["revision"].as_u64()
    });
    assert_eq!(proposal(&accepted, &kerf_id).unwrap()["status"], "accepted");
    let stale = proposal(&accepted, &agent_id).unwrap();
    assert_eq!(stale["status"], "pending");
    assert_eq!(stale["freshness"], "stale");
    let resolved = editor.expect("the note resolved", |event| {
        event["type"] == "comments"
            && event["items"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["resolved"] == true))
    });
    assert_eq!(
        resolved["items"][0]["resolvedRevision"],
        accepted["revision"]
    );

    // The agent revises its stale proposal: version 2 on the new revision.
    client
        .stage_script_proposal_by(
            "agent-2",
            accepted["revision"].as_u64().unwrap(),
            "marker add at=2s name=\"two, again\"",
            None,
            &agent,
            Some(&agent_id),
        )
        .unwrap();
    let revised = editor.state("version 2", |snapshot| {
        snapshot["proposals"].as_array().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry["supersedes"] == agent_id.as_str())
        })
    });
    let v2 = revised["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["supersedes"] == agent_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(v2["version"], 2);
    assert_eq!(v2["freshness"], "current");
    let versions: Vec<&Value> = v2["versions"].as_array().unwrap().iter().collect();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0]["proposalId"], agent_id.as_str());
    assert_eq!(versions[1]["proposalId"], v2["proposalId"]);
    let v2_id = v2["proposalId"].as_str().unwrap().to_owned();

    // Its preview plays in place of the program, until the editor leaves it.
    editor.forget("preview");
    editor.send(json!({"type": "preview", "proposalId": v2_id}));
    editor.expect("the preview rendering", |event| {
        event["type"] == "preview" && event["status"] == "running"
    });
    let shown = editor.expect("the preview shown", |event| {
        event["type"] == "preview"
            && (event["proposalId"] == v2_id.as_str() && event["status"] == "ready"
                || ["required", "failed"].contains(&event["status"].as_str().unwrap_or("")))
    });
    assert_eq!(shown["status"], "ready", "{shown}");
    assert_eq!(shown["proposalId"], v2_id.as_str());
    editor.forget("viewer_reset");
    editor.send(json!({"type": "preview_exit"}));
    editor.expect("the preview left", |event| {
        event["type"] == "preview" && event["status"] == "exited"
    });
    editor.expect("the program again", |event| event["type"] == "viewer_reset");

    // A range preview stopped while it renders, and a stop with none running.
    editor.send(json!({"type": "preview", "start": 0.0, "duration": 4.0}));
    editor.expect("the range rendering", |event| {
        event["type"] == "preview" && event["status"] == "running"
    });
    editor.send(json!({"type": "preview_stop"}));
    editor.expect("the render stopped", |event| {
        event["type"] == "preview" && event["status"] == "stopped"
    });
    editor.send(json!({"type": "preview_stop"}));
    editor.expect("a stop with nothing running", |event| {
        event["type"] == "preview" && event["status"] == "stopped"
    });
}

fn track<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["timeline"]["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap_or_else(|| panic!("no track {id}: {}", snapshot["document"]))
}

fn edit(editor: &mut Editor, snapshot: &Value, ops: Value, what: &str) -> Value {
    editor.send(json!({"type": "edit", "revision": snapshot["revision"], "ops": ops}));
    editor.state(what, |next| {
        next["revision"].as_u64() > snapshot["revision"].as_u64()
    })
}

#[test]
fn titles_go_in_as_generators_on_a_sequence_whose_rate_was_set_while_empty() {
    let project = Project::new("titles");
    let mut editor = Editor::open(&project.0);
    ready(&mut editor);
    let empty = editor.state("the empty project", |_| true);
    assert_eq!(empty["timeline"]["fps"], 30.0);

    // While nothing is in it, the sequence takes a new rate, size and name.
    let spot = edit(
        &mut editor,
        &empty,
        json!([{"op": "set-sequence", "rate": {"num": 25, "den": 1},
                "width": 640, "height": 360, "name": "Spot"}]),
        "25 fps",
    );
    assert_eq!(spot["timeline"]["fps"], 25.0);
    assert_eq!(spot["timeline"]["sequences"][0]["name"], "Spot");
    assert!(spot["document"]
        .as_str()
        .unwrap()
        .contains("sequence s1 name=\"Spot\" rate=25 size=640x360"));
    let ntsc = edit(
        &mut editor,
        &spot,
        json!([{"op": "set-sequence", "rate": "29.97"}]),
        "29.97",
    );
    assert!((ntsc["timeline"]["fps"].as_f64().unwrap() - 30_000.0 / 1001.0).abs() < 1e-9);
    let spot = edit(
        &mut editor,
        &ntsc,
        json!([{"op": "set-sequence", "rate": 25}]),
        "25 again",
    );

    // A title as the Titles browser adds it: a new track, and the
    // generator over what is there, in one edit.
    let titled = edit(
        &mut editor,
        &spot,
        json!([
            {"op": "add-track", "kind": "video", "id": "V2"},
            {"op": "insert", "generator": {"fx": "title", "params": {"text": "Atlas Coffee", "size": 0.2}},
             "at": 0, "frames": 50, "tracks": ["V2"], "mode": "overwrite"}
        ]),
        "the title",
    );
    let clips = track(&titled, "V2")["clips"].as_array().unwrap().clone();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0]["durationFrames"], 50);
    let document = titled["document"].as_str().unwrap();
    assert!(
        document.contains("gen=title@1") && document.contains("text=\"Atlas Coffee\""),
        "{document}"
    );
    // A solid inserted before it ripples it along.
    let solid = edit(
        &mut editor,
        &titled,
        json!([{"op": "insert", "generator": {"fx": "solid", "params": {"color": "#ff0000"}},
                "at": 0, "frames": 25, "tracks": ["V2"]}]),
        "the solid",
    );
    let starts: Vec<&Value> = track(&solid, "V2")["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|clip| &clip["startFrame"])
        .collect();
    assert_eq!(starts, [&json!(0), &json!(25)]);
    assert!((solid["duration"].as_f64().unwrap() - 3.0).abs() < 1e-9);

    // Refused: an effect as a generator, and a rate once there are clips.
    editor.send(json!({"type": "edit", "revision": solid["revision"], "ops": [
        {"op": "insert", "generator": {"fx": "color.basic"}, "at": 0, "frames": 25, "tracks": ["V2"]}]}));
    let refused = editor.expect("an effect refused", |event| event["type"] == "error");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("not a generator"),
        "{refused}"
    );
    editor.send(
        json!({"type": "edit", "revision": solid["revision"], "ops": [
        {"op": "set-sequence", "rate": 30}]}),
    );
    let refused = editor.expect("the rate refused", |event| event["type"] == "error");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .ends_with(kerf_runtime::RATE_NEEDS_EMPTY),
        "{refused}"
    );

    // Playback draws them: the solid red, then the title's white text.
    let (red, width, height) = editor.frame_at(0.5);
    assert_eq!((width, height), (640, 360));
    assert!(red
        .chunks_exact(4)
        .step_by(97)
        .all(|pixel| pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40));
    let (text, _, _) = editor.frame_at(2.0);
    let lit = text
        .chunks_exact(4)
        .filter(|pixel| pixel[..3].iter().all(|value| *value > 200))
        .count();
    assert!(lit > 0 && lit < text.len() / 16, "{lit}");
}
