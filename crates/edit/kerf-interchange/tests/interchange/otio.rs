//! The native `.otio` writer and reader: a project written and read back, a
//! golden file, a file as another editor writes one, and, where Python has
//! OpenTimelineIO, the same files through the bridge.
#![expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]

use kerf_interchange::otio::{self, Item, Timeline, WireKind};
use kerf_interchange::{export, import_otio, otio_json, render, Format, NATIVE_OTIO};
use kerf_lang::{parse_document, print_document, PrintOptions};
use kerf_model::{Catalog, Project};
use serde_json::Value;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What an `.otio` file carries of a project, all of it: a vertical canvas,
/// timecode, a gap, named and disabled tracks and clips, speeds, a dissolve
/// and a crossfade, and markers with and without a color.
const CARRIED: &str = r#"kerf 1
project name="Short Film"

media m1 path="/media/a.mov" rate=23.976 tc=01:00:00:00 dur=00:01:00:00 video=1080x1920 audio=2ch@48000
media m2 path="/media/Score Final.wav" rate=23.976 dur=00:03:00:00 audio=2ch@48000

sequence cut name="Main Edit" rate=23.976 size=1080x1920 audio=48000 start=01:00:00:00
  track V1 video name="Picture"
    clip c1 media=m1 src=01:00:10:00 at=01:00:00:00 dur=00:00:02:00
    clip c2 media=m1 src=01:00:20:00 at=01:00:02:00 dur=00:00:03:00 name="Wide"
    clip c3 media=m1 src=01:00:30:00 at=01:00:06:00 dur=00:00:02:00 speed=2
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
  track V2 video enabled=false
    clip c4 media=m1 src=01:00:40:00 at=01:00:01:00 dur=00:00:01:00 enabled=false
  track A1 audio
    clip a1 media=m1 src=01:00:10:00 at=01:00:00:00 dur=00:00:02:00
    clip a2 media=m1 src=01:00:20:00 at=01:00:02:00 dur=00:00:02:00
    clip a3 media=m2 src=00:00:05:00 at=01:00:05:00 dur=00:00:02:00 speed=0.5
    transition t2 from=a1 to=a2 fx=crossfade@1 dur=00:00:00:08 align=start
  marker mk1 at=01:00:01:00 name="beat" color=red note="hit here"
  marker mk2 at=01:00:04:00 dur=00:00:01:00
"#;

/// A program with what an `.otio` file leaves out beside what it carries.
const PROGRAM: &str = r#"kerf 1
project name="Short Film"

media m1 path="footage/A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000 reel="A001"
media m2 path="music/Score Final.wav" rate=23.976 dur=00:03:00:00 audio=2ch@48000
media m3 path="B002_C010-graded.mov" rate=25 tc=10:00:00:00 dur=00:01:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main Edit" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
    clip c2 media=m3 src=10:00:10:00 at=01:00:06:12 dur=00:00:04:00 name="Wide"
      set opacity=0.5
    clip c3 media=m1 src=14:23:00:00 at=01:00:10:12 dur=00:00:02:00 speed=-2
    clip c4 media=m1 src=14:23:10:00 at=01:00:12:12 dur=00:00:02:00
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
    transition t2 from=c3 to=c4 fx=wipe@1 dur=00:00:00:08
  track V2 video role=titles
    clip t9 gen=title@1 at=01:00:01:00 dur=00:00:02:00 name="Lower third"
      set text="Day One"
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
    clip a3 media=m1 src=14:23:10:00 at=01:00:12:12 dur=00:00:02:00
  track A2 audio role=music muted=true
    clip a2 media=m2 src=00:00:10:00 at=01:00:00:00 dur=00:00:20:00
  track S1 subtitle
    clip cap1 caption="Where are we?" at=01:00:01:00 dur=00:00:02:00
  marker mk1 at=01:00:05:00 name="pickup needed" color=red note="ADR line 4"
  marker mk2 at=01:00:30:00 color=purple
"#;

fn project(text: &str) -> Project {
    parse_document(text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
        .project
}

fn printed(project: &Project) -> String {
    print_document(project, Catalog::builtin(), PrintOptions::default())
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

#[test]
fn a_project_written_and_read_back_is_the_same_project() {
    let original = project(CARRIED);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("out/cut.otio");
    let written = export(&original, None, &path, Format::Otio).unwrap();
    assert_eq!(written.adapter, NATIVE_OTIO);
    // The frame size is in the file for Kerf alone: another editor still
    // does not get it, and that is all this project loses.
    let codes: Vec<&str> = written
        .report
        .losses
        .iter()
        .map(|i| i.code.as_str())
        .collect();
    assert_eq!(codes, ["canvas_not_exported"]);

    let read = import_otio(&path).unwrap();
    assert_eq!(printed(&read.project), printed(&original));
    assert_eq!(read.project, original);
    // Nothing was lost on the way in, and the canvas was not guessed.
    assert!(read.report.losses.is_empty(), "{:#?}", read.report.losses);
    let warnings: Vec<&str> = read
        .report
        .warnings
        .iter()
        .map(|i| i.code.as_str())
        .collect();
    assert_eq!(warnings, ["media_assumed"]);
}

#[test]
fn a_program_matches_the_golden_otio_file_and_lists_what_it_leaves_out() {
    let project = project(PROGRAM);
    let (text, issues) = render(&project, &project.sequences[0], Format::Otio).unwrap();
    let golden = golden("program.otio");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &text).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(
        text, expected,
        "regenerate with UPDATE_GOLDEN=1 cargo test -p kerf-interchange --test interchange otio::"
    );
    let left_out: Vec<(&str, Option<&str>)> = issues
        .iter()
        .map(|issue| (issue.code.as_str(), issue.path.as_deref()))
        .collect();
    assert_eq!(
        left_out,
        [
            ("canvas_not_exported", Some("s1")),
            ("links_not_exported", Some("s1")),
            ("properties_not_exported", Some("c2")),
            ("reverse_not_exported", Some("c3")),
            ("transition_not_exported", Some("t2")),
            ("content_not_exported", Some("t9")),
            ("track_state_not_exported", Some("A2")),
            ("captions_not_exported", Some("S1")),
        ],
        "{issues:#?}"
    );
    let said = |code: &str| {
        let issue = issues.iter().find(|issue| issue.code == code).unwrap();
        issue.message.as_str()
    };
    assert_eq!(
        said("transition_not_exported"),
        "`wipe` was exported as a dissolve"
    );
    assert_eq!(
        said("properties_not_exported"),
        "not exported: properties (opacity)"
    );
    assert!(said("content_not_exported").contains("`title` generator"));
    assert!(said("captions_not_exported").contains(".srt"));

    // The golden file holds the schemas OpenTimelineIO documents, and no other.
    let value: Value = serde_json::from_str(&text).unwrap();
    let mut schemas = Vec::new();
    collect_schemas(&value, &mut schemas);
    schemas.sort();
    schemas.dedup();
    assert_eq!(
        schemas,
        [
            "Clip.2",
            "ExternalReference.1",
            "Gap.1",
            "LinearTimeWarp.1",
            "Marker.2",
            "RationalTime.1",
            "Stack.1",
            "TimeRange.1",
            "Timeline.1",
            "Track.1",
            "Transition.1",
        ]
    );
}

fn collect_schemas(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            if let Some(Value::String(schema)) = fields.get("OTIO_SCHEMA") {
                into.push(schema.clone());
            }
            fields
                .values()
                .for_each(|value| collect_schemas(value, into));
        }
        Value::Array(items) => items.iter().for_each(|value| collect_schemas(value, into)),
        _ => {}
    }
}

#[test]
fn clips_of_another_rate_keep_their_cuts() {
    // 25 fps footage on a 23.976 timeline: 4 seconds is 100 of its frames
    // and a little more, which the file says rather than round away.
    let project = project(PROGRAM);
    let (timeline, _) = otio::to_timeline(&project).unwrap();
    let (read, issues) = otio_json::read(&otio_json::write(&timeline).unwrap()).unwrap();
    assert_eq!(read, timeline);
    assert!(issues.is_empty(), "{issues:#?}");
    let Item::Clip {
        start, duration, ..
    } = read.tracks[0].items[2]
    else {
        panic!("{:?}", read.tracks[0].items[2]);
    };
    assert_eq!((start, duration), (156, 96));
    let Item::Clip { start, speed, .. } = read.tracks[0].items[3] else {
        panic!("{:?}", read.tracks[0].items[3]);
    };
    assert_eq!((start, speed), (252, Some(2.0)));
}

#[test]
fn what_a_whole_sequence_loses_is_listed_too() {
    let project = project(
        r#"kerf 1
project name="Losses"
media m1 path="/m/a.mov" rate=29.97 dur=00:01:00:00 video=1920x1080 audio=2ch@48000
media m2 path="/m/unused.mov" rate=29.97 dur=00:01:00:00 video=1920x1080
sequence s1 name="HDR" rate=29.97 size=1920x1080 color=rec2100-pq start=00:00:59;00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=00:00:59;00 dur=00:00:02:00 speed={0f:1 60f:3}
sequence s2 name="Other" rate=29.97 size=1920x1080
"#,
    );
    let codes = |format| -> Vec<String> {
        let (_, issues) = render(&project, &project.sequences[0], format).unwrap();
        issues.into_iter().map(|issue| issue.code).collect()
    };
    assert_eq!(
        codes(Format::Otio),
        [
            "sequences_not_exported",
            "canvas_not_exported",
            "drop_frame_not_exported",
            "color_space_not_exported",
            "speed_ramp_not_exported",
            "media_not_exported",
        ]
    );
    // FCPXML has the frame size and says drop-frame; it names files by the
    // clips that play them, so one nothing plays is not its to list.
    assert_eq!(
        codes(Format::Fcpxml),
        [
            "sequences_not_exported",
            "color_space_not_exported",
            "speed_ramp_not_exported"
        ]
    );
    // A ramp from 1 to 3 plays as fast as 2 overall, and the file says 2.
    let (timeline, _) = otio::sequence_timeline(&project, &project.sequences[0]);
    assert!(
        matches!(timeline.tracks[0].items[0], Item::Clip { speed: Some(speed), .. } if speed == 2.0)
    );
    assert_eq!(
        timeline
            .sequence
            .as_ref()
            .map(|sequence| sequence.drop_frame),
        Some(true)
    );
    // The second sequence is written when it is the one asked for.
    let (timeline, _) = otio::sequence_timeline(&project, &project.sequences[1]);
    assert_eq!(timeline.sequence.unwrap().name.as_deref(), Some("Other"));
}

/// A file as another editor writes one: no start time, `Clip.1` with its one
/// media reference as a `file:` URL, a `Marker.3` color, a nested stack, and
/// effects.
const FOREIGN: &str = r#"{
  "OTIO_SCHEMA": "Timeline.1",
  "name": "Conform",
  "tracks": {
    "OTIO_SCHEMA": "Stack.1",
    "children": [
      {
        "OTIO_SCHEMA": "Track.1",
        "kind": "Video",
        "name": "Video 1",
        "markers": [
          {"OTIO_SCHEMA": "Marker.3", "name": "fix", "comment": "too dark",
           "color": {"OTIO_SCHEMA": "Color.1", "name": "BLUE", "r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0},
           "marked_range": {"OTIO_SCHEMA": "TimeRange.1",
             "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 10.0},
             "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 0.0}}}
        ],
        "children": [
          {"OTIO_SCHEMA": "Gap.1",
           "source_range": {"OTIO_SCHEMA": "TimeRange.1",
             "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 0.0},
             "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 25.0}}},
          {"OTIO_SCHEMA": "Clip.1", "name": "A002.mov",
           "source_range": {"OTIO_SCHEMA": "TimeRange.1",
             "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 100.0},
             "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 50.0}},
           "effects": [
             {"OTIO_SCHEMA": "LinearTimeWarp.1", "effect_name": "LinearTimeWarp", "time_scalar": 0.5},
             {"OTIO_SCHEMA": "Effect.1", "effect_name": "Blur"}
           ],
           "markers": [
             {"OTIO_SCHEMA": "Marker.2", "name": "", "color": "PINK",
              "marked_range": {"OTIO_SCHEMA": "TimeRange.1",
                "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 110.0},
                "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 5.0}}}
           ],
           "media_reference": {"OTIO_SCHEMA": "ExternalReference.1",
             "target_url": "file:///shoot/Day%201/A002.mov",
             "available_range": {"OTIO_SCHEMA": "TimeRange.1",
               "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 0.0},
               "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 1500.0}}}},
          {"OTIO_SCHEMA": "Transition.1", "transition_type": "SMPTE_Dissolve",
           "in_offset": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 5.0},
           "out_offset": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 5.0}},
          {"OTIO_SCHEMA": "Clip.2", "name": "Closer", "enabled": false,
           "source_range": {"OTIO_SCHEMA": "TimeRange.1",
             "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 400.0},
             "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 25.0}},
           "media_references": {"DEFAULT_MEDIA": {"OTIO_SCHEMA": "ExternalReference.1",
             "target_url": "file:///shoot/Day%201/A002.mov"}},
           "active_media_reference_key": "DEFAULT_MEDIA"},
          {"OTIO_SCHEMA": "Stack.1", "name": "Compound",
           "children": [{"OTIO_SCHEMA": "Track.1", "kind": "Video", "children": [
             {"OTIO_SCHEMA": "Gap.1",
              "source_range": {"OTIO_SCHEMA": "TimeRange.1",
                "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 0.0},
                "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 10.0}}}]}]},
          {"OTIO_SCHEMA": "Clip.2", "name": "",
           "source_range": {"OTIO_SCHEMA": "TimeRange.1",
             "start_time": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 0.0},
             "duration": {"OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 20.0}},
           "media_references": {"DEFAULT_MEDIA": {"OTIO_SCHEMA": "MissingReference.1"}},
           "active_media_reference_key": "DEFAULT_MEDIA"}
        ]
      },
      {"OTIO_SCHEMA": "Track.1", "kind": "Subtitles", "children": []}
    ]
  }
}"#;

#[test]
fn a_file_from_another_editor_is_read_with_what_it_loses() {
    let (timeline, issues) = otio_json::read(FOREIGN).unwrap();
    // With no start time it counts at the rate of the first thing on it.
    assert_eq!(
        (timeline.name.as_str(), timeline.rate, timeline.start),
        ("Conform", 25.0, 0)
    );
    assert_eq!(timeline.sequence, None);
    assert_eq!(timeline.media.len(), 2);
    assert_eq!(timeline.media[0].path, "/shoot/Day 1/A002.mov");
    assert_eq!(timeline.media[0].duration, Some(1500));
    assert_eq!(timeline.media[1].path, "missing-2");
    let track = &timeline.tracks[0];
    assert_eq!(
        (track.kind, track.name.as_deref()),
        (WireKind::Video, Some("Video 1"))
    );
    assert_eq!(
        track.items,
        [
            // Named after its file: no name of its own.
            Item::Clip {
                id: None,
                name: None,
                media: 0,
                source_in: 100,
                start: 25,
                duration: 50,
                speed: Some(0.5),
                enabled: true,
            },
            Item::Transition {
                id: None,
                before: 5,
                after: 5
            },
            Item::Clip {
                id: None,
                name: Some("Closer".into()),
                media: 0,
                source_in: 400,
                start: 75,
                duration: 25,
                speed: None,
                enabled: false,
            },
            // After the nested stack, which takes its ten frames.
            Item::Clip {
                id: None,
                name: None,
                media: 1,
                source_in: 0,
                start: 110,
                duration: 20,
                speed: None,
                enabled: true,
            },
        ]
    );
    // The clip's marker, on the timeline: 10 frames into a clip at 25.
    let markers: Vec<_> = timeline
        .markers
        .iter()
        .map(|m| {
            (
                m.at,
                m.duration,
                m.name.as_deref(),
                m.color.as_deref(),
                m.note.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        markers,
        [
            (35, 5, None, Some("pink"), None),
            (10, 0, Some("fix"), Some("blue"), Some("too dark")),
        ]
    );
    let lost: Vec<(&str, Option<&str>)> = issues
        .iter()
        .map(|issue| (issue.code.as_str(), issue.path.as_deref()))
        .collect();
    assert_eq!(
        lost,
        [
            ("effects_not_imported", Some("tracks[0].children[1]")),
            ("item_not_imported", Some("tracks[0].children[4]")),
            ("missing_media_reference", Some("tracks[0].children[5]")),
            ("track_not_imported", Some("tracks[1]")),
        ]
    );

    // And as a project: 25 fps, the canvas assumed, the half speed kept.
    let (project, issues) = otio::from_timeline(&timeline).unwrap();
    let text = printed(&project);
    assert!(
        text.contains("sequence s1 name=\"Conform\" rate=25 size=1920x1080"),
        "{text}"
    );
    assert!(text.contains("speed=0.5"), "{text}");
    assert!(text.contains("fx=dissolve@1 dur=00:00:00:10"), "{text}");
    assert!(issues.iter().any(|issue| issue.code == "canvas_assumed"));
}

// ---------------------------------------------------------------------------
// The Python bridge, where it can run
// ---------------------------------------------------------------------------

/// The Python that has OpenTimelineIO: `KERF_OTIO_PYTHON`, else `python3`.
/// `None` where it has none, and the bridge checks are skipped.
fn bridge_python() -> Option<PathBuf> {
    let python = std::env::var_os("KERF_OTIO_PYTHON")
        .map_or_else(|| PathBuf::from("python3"), PathBuf::from);
    let has = Command::new(&python)
        .args(["-c", "import opentimelineio"])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !has {
        eprintln!(
            "skipped: {} has no opentimelineio (set KERF_OTIO_PYTHON to one that has)",
            python.display()
        );
    }
    has.then_some(python)
}

fn bridge(python: &Path, args: &[&str], stdin: Option<&str>) -> Value {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/interchange/otio_bridge.py");
    #[expect(
        clippy::disallowed_methods,
        reason = "the test's OpenTimelineIO script, waited on"
    )]
    let mut child = Command::new(python)
        .arg(script)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.unwrap_or_default().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["ok"], true, "{answer}");
    answer
}

#[test]
fn the_bridge_reads_what_is_written_here_and_writes_what_is_read_here() {
    let Some(python) = bridge_python() else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    for (name, document) in [("carried", CARRIED), ("program", PROGRAM)] {
        let project = project(document);
        let (timeline, _) = otio::to_timeline(&project).unwrap();

        // Written here, read by OpenTimelineIO: the same timeline.
        let native = folder.path().join(format!("{name}-native.otio"));
        std::fs::write(&native, otio_json::write(&timeline).unwrap()).unwrap();
        let answer = bridge(
            &python,
            &[
                "import",
                "--input",
                native.to_str().unwrap(),
                "--adapter",
                "otio_json",
            ],
            None,
        );
        let through: Timeline = serde_json::from_value(answer["timeline"].clone()).unwrap();
        assert_eq!(
            through, timeline,
            "{name}: the bridge read another timeline"
        );
        assert_eq!(answer["report"]["issues"], serde_json::json!([]), "{name}");

        // Written by OpenTimelineIO, read here: the same timeline.
        let bridged = folder.path().join(format!("{name}-bridge.otio"));
        bridge(
            &python,
            &[
                "export",
                "--output",
                bridged.to_str().unwrap(),
                "--adapter",
                "otio_json",
            ],
            Some(&serde_json::json!({ "timeline": timeline }).to_string()),
        );
        let (read, issues) = otio_json::read(&std::fs::read_to_string(&bridged).unwrap()).unwrap();
        assert_eq!(
            read, timeline,
            "{name}: read another timeline than the bridge wrote"
        );
        assert!(issues.is_empty(), "{name}: {issues:#?}");

        // And OpenTimelineIO holds the two files to be the same timeline.
        let same = Command::new(&python)
            .args([
                "-c",
                "import sys, opentimelineio as otio\n\
                 a, b = (otio.adapters.read_from_file(path) for path in sys.argv[1:3])\n\
                 sys.exit(0 if a.is_equivalent_to(b) else 1)",
            ])
            .args([&native, &bridged])
            .status()
            .unwrap();
        assert!(
            same.success(),
            "{name}: OpenTimelineIO reads the two files differently"
        );
    }

    // A file another editor wrote reads the same either way. Its `Marker.3`
    // is of an OpenTimelineIO not yet released, which a released one refuses
    // to read: for the bridge it is the `Marker.2` it would be.
    let foreign = folder.path().join("foreign.otio");
    let released = FOREIGN.replace("Marker.3", "Marker.2").replace(
        r#"{"OTIO_SCHEMA": "Color.1", "name": "BLUE", "r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0}"#,
        "\"BLUE\"",
    );
    assert_ne!(released, FOREIGN);
    std::fs::write(&foreign, released).unwrap();
    let answer = bridge(
        &python,
        &[
            "import",
            "--input",
            foreign.to_str().unwrap(),
            "--adapter",
            "otio_json",
        ],
        None,
    );
    let through: Timeline = serde_json::from_value(answer["timeline"].clone()).unwrap();
    let (read, issues) = otio_json::read(FOREIGN).unwrap();
    assert_eq!(through, read);
    let codes = |issues: &Value| -> Vec<String> {
        let mut codes: Vec<String> = issues
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| {
                format!(
                    "{} {}",
                    issue["code"].as_str().unwrap(),
                    issue["path"].as_str().unwrap()
                )
            })
            .collect();
        codes.sort();
        codes
    };
    let mut here: Vec<String> = issues
        .iter()
        .map(|issue| format!("{} {}", issue.code, issue.path.as_deref().unwrap()))
        .collect();
    here.sort();
    assert_eq!(codes(&answer["report"]["issues"]), here);
}
