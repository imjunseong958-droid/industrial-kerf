//! The native CMX3600 writer against a golden file and the rules readers
//! hold it to.

use kerf_interchange::edl::{locator_color, marker_locators, to_edl, EdlError, Locator};
use kerf_lang::parse_document;
use kerf_model::{Catalog, MarkerColor, Project, Rate, Time};
use std::path::Path;

const EDIT: &str = r#"kerf 1
project name="Short Film"

media m1 path="footage/A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000 reel="A001"
media m2 path="music/Score Final.wav" rate=23.976 dur=00:03:00:00 audio=2ch@48000
media m3 path="B002_C010-graded.mov" rate=23.976 tc=10:00:00:00 dur=00:01:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main Edit" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
    clip c2 media=m3 src=10:00:10:00 at=01:00:06:12 dur=00:00:04:00 name="Wide"
    clip c3 media=m1 src=14:23:00:00 at=01:00:10:12 dur=00:00:02:00 speed=2
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
  track V2 video role=titles
    clip t9 gen=title@1 at=01:00:01:00 dur=00:00:02:00 name="Lower third"
      set text="Day One"
  track V3 video enabled=false
    clip c9 media=m3 src=10:00:00:00 at=01:00:00:00 dur=00:00:01:00
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
  track A2 audio role=music
    clip a2 media=m2 src=00:00:10:00 at=01:00:00:00 dur=00:00:20:00
  track A3 audio muted=true
    clip a3 media=m2 src=00:00:00:00 at=01:00:00:00 dur=00:00:01:00
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

fn f(rate: Rate, frames: i64) -> Time {
    rate.time_of(frames)
}

#[test]
fn a_program_matches_the_golden_edl() {
    let project = project(EDIT);
    let sequence = &project.sequences[0];
    let mut locators = marker_locators(sequence);
    // A review comment, as the bridge hands it over, with a line break.
    locators.push(Locator::from((
        f(Rate::FPS_23_976, 24 * 11),
        "yellow",
        "Caption sits on\nthe logo",
    )));
    let edl = to_edl(&project, sequence, &locators).unwrap();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/program.edl");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &edl).unwrap();
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(
        edl, expected,
        "regenerate with UPDATE_GOLDEN=1 cargo test -p kerf-interchange --test edl"
    );
}

#[test]
fn every_event_line_has_the_cmx3600_columns() {
    let project = project(EDIT);
    let edl = to_edl(&project, &project.sequences[0], &[]).unwrap();
    assert!(edl.contains("\nM2   A001           048.0                14:23:00:00\n"));
    for line in edl
        .lines()
        .filter(|line| line.starts_with(char::is_numeric))
    {
        // Event number, reel, channel, transition, then four timecodes,
        // each at a fixed column.
        assert_eq!(&line[3..5], "  ", "{line}");
        assert_eq!(&line[13..14], " ", "{line}");
        assert_eq!(&line[19..20], " ", "{line}");
        assert_eq!(&line[28..29], " ", "{line}");
        let codes: Vec<&str> = line[29..].split(' ').collect();
        assert_eq!(codes.len(), 4, "{line}");
        assert!(codes.iter().all(|tc| tc.len() == 11), "{line}");
    }
}

#[test]
fn drop_frame_sequences_say_so_and_count_drop_frame() {
    let text = "kerf 1
project name=\"NTSC\"
media m1 path=\"a.mov\" rate=29.97 tc=00:00:59;00 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
sequence s1 name=\"Broadcast\" rate=29.97 size=1920x1080 start=00:00:59;00
  track V1 video
    clip c1 media=m1 src=00:00:59;00 at=00:00:59;00 dur=00:00:02:00
";
    let dropped = project(text);
    let edl = to_edl(&dropped, &dropped.sequences[0], &[]).unwrap();
    assert_eq!(
        edl,
        "TITLE: Broadcast
FCM: DROP FRAME

001  a        V     C        00:00:59;00 00:01:01;02 00:00:59;00 00:01:01;02
* FROM CLIP NAME: a.mov
* SOURCE FILE: a.mov
"
    );
    // The same edit, non-drop.
    let ndf = project(&text.replace(";", ":"));
    let edl = to_edl(&ndf, &ndf.sequences[0], &[]).unwrap();
    assert!(edl.contains("FCM: NON-DROP FRAME\n"), "{edl}");
    assert!(
        edl.contains("00:00:59:00 00:01:01:00 00:00:59:00 00:01:01:00"),
        "{edl}"
    );
}

#[test]
fn every_listed_rate_writes_and_others_are_refused() {
    for (rate, frames) in [
        ("23.976", 24),
        ("24", 24),
        ("25", 25),
        ("29.97", 30),
        ("30", 30),
        ("50", 50),
        ("59.94", 60),
        ("60", 60),
    ] {
        let text = format!(
            "kerf 1\nproject name=\"r\"\n\
             media m1 path=\"a.mov\" rate={rate} dur=00:01:00:00 video=64x36\n\
             sequence s1 name=\"R\" rate={rate} size=64x36\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:01:00\n"
        );
        let project = project(&text);
        let edl = to_edl(&project, &project.sequences[0], &[]).unwrap();
        assert!(
            edl.contains("00:00:00:00 00:00:01:00 00:00:00:00 00:00:01:00"),
            "{rate} ({frames} frames a second): {edl}"
        );
    }
    let text = "kerf 1\nproject name=\"r\"\n\
                media m1 path=\"a.mov\" rate=12 dur=00:01:00:00 video=64x36\n\
                sequence s1 rate=12 size=64x36\n";
    let project = project(text);
    let error = to_edl(&project, &project.sequences[0], &[]).unwrap_err();
    assert_eq!(error, EdlError::UnsupportedRate(Rate::new(12, 1).unwrap()));
    assert!(error.to_string().contains("23.976"), "{error}");
}

#[test]
fn locators_land_under_their_event_or_after_the_header() {
    let text = "kerf 1
project name=\"L\"
media m1 path=\"/shoot/Day 1/A002.mov\" rate=25 dur=00:01:00:00 video=64x36
sequence s1 name=\"L\" rate=25 size=64x36 start=10:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:00:00 at=10:00:02:00 dur=00:00:01:00
    clip c2 media=m1 src=00:00:10:00 at=10:00:04:00 dur=00:00:01:00
";
    let project = project(text);
    let rate = Rate::FPS_25;
    let locators = [
        Locator::new(f(rate, 25), "", "before anything"),
        Locator::new(f(rate, 60), "green", "in the first"),
        Locator::new(f(rate, 80), "Blue", "in the gap"),
        Locator::new(f(rate, 110), "cyan", "in the second"),
    ];
    let edl = to_edl(&project, &project.sequences[0], &locators).unwrap();
    assert_eq!(
        edl,
        "TITLE: L
FCM: NON-DROP FRAME

* LOC: 10:00:01:00 RED     before anything

001  A002     V     C        00:00:00:00 00:00:01:00 10:00:02:00 10:00:03:00
* FROM CLIP NAME: A002.mov
* SOURCE FILE: A002.mov
* LOC: 10:00:02:10 GREEN   in the first
* LOC: 10:00:03:05 BLUE    in the gap

002  A002     V     C        00:00:10:00 00:00:11:00 10:00:04:00 10:00:05:00
* FROM CLIP NAME: A002.mov
* SOURCE FILE: A002.mov
* LOC: 10:00:04:10 CYAN    in the second
"
    );
}

#[test]
fn marker_colours_map_to_avid_locator_colours() {
    assert_eq!(locator_color(None), "RED");
    assert_eq!(locator_color(Some(MarkerColor::Orange)), "RED");
    assert_eq!(locator_color(Some(MarkerColor::Pink)), "MAGENTA");
    assert_eq!(locator_color(Some(MarkerColor::White)), "WHITE");
    let project = project(EDIT);
    let locators = marker_locators(&project.sequences[0]);
    assert_eq!(
        locators,
        [
            Locator::new(
                f(Rate::FPS_23_976, 120),
                "RED",
                "pickup needed - ADR line 4"
            ),
            Locator::new(f(Rate::FPS_23_976, 720), "MAGENTA", ""),
        ]
    );
}
