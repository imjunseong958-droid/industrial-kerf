//! A preset that carries a project's look, applied to a project without it
//! on music whose beats its marker moves to, against a real kerfd. A file of
//! its own: the process's catalog has one project's looks at a time.

use kerf_mcp::KerfMcpRouter;
use kerf_model::Catalog;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const MINE: &str = r#"kerf-look 1
look look.test.mine version=1 name="Mine" category="Test" on=clip
  doc "Nudges the picture at a hit."
  param hit time default=0f
  set x={$hit:0 $hit+2f:0.1 $hit+4f:0}
"#;

/// Four seconds of picture over beeps every half second.
/// A router on `directory` for a test. What it lets an agent do is the default, every edit proposed
/// and no making run, whatever the person running the tests lets their own agents do in their
/// `~/.kerf/agent.json`: a test that wants another way says so with `set_permissions`.
fn router_on(directory: &Path, kerfd: Option<&Path>) -> KerfMcpRouter {
    let router = KerfMcpRouter::new(directory, kerfd).unwrap();
    router.set_permissions(Some(kerf_mcp::permissions::Permissions::default()));
    router
}

fn router() -> Option<(TempDir, KerfMcpRouter)> {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipping: ffmpeg is not installed");
        return None;
    }
    let directory = TempDir::new().unwrap();
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:size=320x180:rate=30:duration=4",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "aevalsrc=if(lt(mod(t\\,0.5)\\,0.05)\\,sin(2*PI*880*t)\\,0):s=48000:d=4",
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-t", "4"])
        .arg(directory.path().join("beats.mp4"))
        .status()
        .expect("ffmpeg");
    assert!(status.success());
    std::fs::write(
        directory.path().join("project.kerf"),
        "kerf 1
project name=\"Beats\"
media m1 path=\"beats.mp4\" rate=30 dur=00:00:04:00 video=320x180 audio=1ch@48000
sequence s1 rate=30 size=320x180
  track V1 video
    clip c1 media=m1 at=00:00:00:00 dur=00:00:04:00 link=L1
  track A1 audio
    clip a1 media=m1 at=00:00:00:00 dur=00:00:04:00 link=L1
",
    )
    .unwrap();
    let kerfd = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/kerfd");
    let router = router_on(directory.path(), Some(&kerfd));
    Some((directory, router))
}

fn ok(router: &KerfMcpRouter, name: &str, arguments: Value) -> Value {
    let response = router.call(name, arguments);
    assert_eq!(response["ok"], true, "{name}: {response}");
    response["result"].clone()
}

fn kept(directory: &Path) -> PathBuf {
    kerf_model::look::project_looks(directory).join("look.test.mine@1.kerflook")
}

#[test]
fn a_preset_brings_its_looks_and_lands_its_markers_on_new_beats() {
    let Some((source_directory, source)) = router() else {
        return;
    };
    // A look added beside a running kerfd is one it edits with.
    let looks = kerf_model::look::project_looks(source_directory.path());
    std::fs::create_dir_all(&looks).unwrap();
    std::fs::write(looks.join("mine.kerflook"), MINE).unwrap();
    let revision = source.runtime().get_state().unwrap().revision;
    source
        .runtime()
        .apply_script_request(
            "ui",
            "preset-look",
            "marker add as=hit1 at=00:00:01:07\nfx c1 add look.test.mine@1 hit=hit1\n",
            revision,
        )
        .unwrap();
    let saved = ok(
        &source,
        "edit",
        json!({"savePreset":"looks.kerfpreset", "name":"Looks"}),
    );
    assert_eq!(saved["looks"], json!(["look.test.mine@1"]));

    let Some((destination_directory, destination)) = router() else {
        return;
    };
    std::fs::copy(
        source_directory.path().join("looks.kerfpreset"),
        destination_directory.path().join("looks.kerfpreset"),
    )
    .unwrap();
    assert!(Catalog::builtin().look("look.test.mine", 1).is_none());
    let inspected = ok(&destination, "look", json!({"preset":"looks.kerfpreset"}));
    assert_eq!(inspected["looks"][0]["id"], "look.test.mine");
    let before = destination.runtime().get_document(false).unwrap();
    let proposed = ok(
        &destination,
        "edit",
        json!({"preset":"looks.kerfpreset",
            "bindings":{"slot1":{"media":"m1","beats":true}}}),
    );
    assert_eq!(proposed["check"]["valid"], true, "{proposed}");
    // The look is the project's now, word for word.
    assert_eq!(
        std::fs::read_to_string(kept(destination_directory.path())).unwrap(),
        MINE
    );
    destination
        .runtime()
        .accept_proposal(
            proposed["proposal"].as_str().unwrap(),
            "ui",
            "accept-looks",
            before.0,
        )
        .unwrap();
    let (_, document) = destination.runtime().get_document(false).unwrap();
    let project = kerf_lang::parse_document(&document, Catalog::builtin())
        .unwrap()
        .project;
    let sequence = &project.sequences[0];
    // 1s 7f is nearer the beep at 1s than the one at 1.5s.
    let marker = &sequence.markers[0];
    let frame = sequence.rate.frames_round(marker.at);
    assert!((29..=32).contains(&frame), "{frame}: {document}");
    assert!(
        document.contains(&format!("fx look.test.mine@1 hit={}", marker.id)),
        "{document}"
    );
}
