//! `angle=`: what it reads, prints, and refuses.

use kerf_lang::{apply_script, parse_document, print_document, print_ops, PrintOptions};
use kerf_model::Catalog;

fn document(angle: &str) -> String {
    format!("kerf 1\nproject name=\"t\"\nmedia m path=\"a.mov\" rate=24 dur=96f video=64x36 audio=2ch@48000\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 seq=mc src=0f at=0f dur=24f{angle}\nsequence mc rate=24 size=64x36 audio=48000\n  track V1 video\n    clip a media=m src=0f at=0f dur=48f\n  track V2 video\n    clip b media=m src=0f at=0f dur=48f\n  track A1 audio\n    clip s media=m src=0f at=0f dur=48f\n")
}

#[test]
fn an_angle_reads_and_prints() {
    let read = parse_document(&document(" angle=V2"), Catalog::builtin()).expect("reads");
    let printed = print_document(&read.project, Catalog::builtin(), PrintOptions::default());
    assert!(printed.contains(" angle=V2\n"), "{printed}");
    assert_eq!(
        parse_document(&printed, Catalog::builtin())
            .expect("reads back")
            .project,
        read.project
    );
    let plain = parse_document(&document(""), Catalog::builtin()).expect("reads");
    let printed = print_document(&plain.project, Catalog::builtin(), PrintOptions::default());
    assert!(!printed.contains("angle"), "{printed}");
}

#[test]
fn an_angle_must_be_a_track_of_the_right_kind() {
    for bad in [" angle=V7", " angle=A1"] {
        let refused = parse_document(&document(bad), Catalog::builtin());
        assert!(refused.is_err(), "{bad}");
    }
    let project = parse_document(&document(""), Catalog::builtin())
        .expect("reads")
        .project;
    assert!(apply_script(&project, Catalog::builtin(), "set c1 angle=V7").is_err());
    // Footage has no tracks to pick from.
    assert!(apply_script(&project, Catalog::builtin(), "use mc\nset a angle=V1").is_err());
}

#[test]
fn switching_prints_as_the_line_it_came_from() {
    let project = parse_document(&document(""), Catalog::builtin())
        .expect("reads")
        .project;
    for line in ["set c1 angle=V2", "set c1 angle=\"\""] {
        let applied = apply_script(&project, Catalog::builtin(), line).expect("runs");
        assert_eq!(
            print_ops(&project, Catalog::builtin(), &applied.ops).trim(),
            line
        );
    }
}

#[test]
fn a_split_keeps_the_angle_on_both_sides() {
    let project = parse_document(&document(" angle=V2"), Catalog::builtin())
        .expect("reads")
        .project;
    let split = apply_script(&project, Catalog::builtin(), "split c1 at=6f as=c2")
        .expect("runs")
        .project;
    assert_eq!(
        split
            .clip("c2")
            .expect("the second part")
            .angle
            .as_ref()
            .map(|a| a.as_str()),
        Some("V2")
    );
}
