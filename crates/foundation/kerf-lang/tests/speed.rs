//! Changing speed from a script: what `speed` does to a clip and how it prints back.

use kerf_lang::{apply_script, parse_document, print_document, print_ops, PrintOptions};
use kerf_model::{Catalog, Clip, Project, Rate};

fn project() -> Project {
    parse_document(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"a.mov\" rate=24 dur=960f video=64x36 audio=2ch@48000\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 media=m src=100f at=0f dur=48f link=L\n    clip c2 media=m src=500f at=96f dur=24f\n  track A1 audio\n    clip a1 media=m src=100f at=0f dur=48f link=L\n",
        Catalog::builtin(),
    )
    .expect("parses")
    .project
}

#[test]
fn a_ramp_keeps_the_source_range_of_the_clip_and_what_is_linked_to_it() {
    let applied =
        apply_script(&project(), Catalog::builtin(), "speed c1 {0f:2 24f:1/2}").expect("runs");
    let clips: Vec<&Clip> = ["c1", "a1"]
        .iter()
        .map(|id| applied.project.clip(id).expect("a clip"))
        .collect();
    for clip in clips {
        let (low, high) = clip.source_range().expect("a source");
        // 30 frames in the first 24, and the other 18 at half speed: 60 frames in all.
        assert_eq!(clip.duration, Rate::FPS_24.time_of(60), "{}", clip.id);
        assert_eq!(low, Rate::FPS_24.time_of(100));
        assert_eq!(high, Rate::FPS_24.time_of(148));
    }
}

#[test]
fn a_ramp_prints_back_as_the_line_it_came_from() {
    let project = project();
    let line = "speed c1 {0f:2:ease 24f:0.5}";
    let applied = apply_script(&project, Catalog::builtin(), line).expect("runs");
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &applied.ops).trim(),
        line
    );
}

#[test]
fn a_ramp_refuses_to_run_into_the_next_clip_as_a_speed_does() {
    let refused = apply_script(&project(), Catalog::builtin(), "speed c1 {0f:1/2 24f:1/4}");
    assert!(refused.is_err());
}

#[test]
fn a_line_inserting_a_ramp_runs_and_prints() {
    let project = project();
    let line = "overwrite media=m src=0f at=200f dur=24f speed={0f:1 12f:2} on=V1 as=c9";
    let applied = apply_script(&project, Catalog::builtin(), line).expect("runs");
    assert_eq!(
        applied
            .project
            .clip("c9")
            .expect("the clip")
            .speed()
            .constant(),
        None
    );
    let printed = print_ops(&project, Catalog::builtin(), &applied.ops);
    assert!(printed.contains("speed={0f:1 12f:2}"), "{printed}");
    apply_script(&project, Catalog::builtin(), &printed).expect("runs again");
}

#[test]
fn splitting_and_trimming_a_ramp_print_what_reads_back_the_same() {
    let project = apply_script(&project(), Catalog::builtin(), "speed c1 {0f:2 24f:1/2}")
        .expect("runs")
        .project;
    let edited = apply_script(
        &project,
        Catalog::builtin(),
        "split c1 at=12f as=c7\ntrim c7 in +2f",
    )
    .expect("runs")
    .project;
    let text = print_document(&edited, Catalog::builtin(), PrintOptions::default());
    assert!(text.contains("speed={-14f:2 10f:0.5}"), "{text}");
    let read = parse_document(&text, Catalog::builtin()).expect("reads back");
    assert_eq!(read.project, edited);
}

#[test]
fn buses_run_from_a_script_and_print_back_as_they_ran() {
    let project = project();
    let script = "bus add music gain=-6\nbus fx music add audio.limiter@1 limit_db=-3\nbus add all\nbus set music to=all gain=-3 reset=pan\ntrack set A1 bus=music\nbus fx music set 1 limit_db=-2\nbus fx music remove 1";
    let applied = apply_script(&project, Catalog::builtin(), script).expect("runs");
    let sequence = &applied.project.sequences[0];
    let music = sequence.bus("music").expect("the bus");
    assert_eq!(music.to.as_ref().map(|to| to.as_str()), Some("all"));
    assert!(music.effects.is_empty());
    assert_eq!(
        sequence
            .track("A1")
            .expect("the track")
            .bus
            .as_ref()
            .map(|b| b.as_str()),
        Some("music")
    );
    let printed = print_ops(&project, Catalog::builtin(), &applied.ops);
    let again = apply_script(&project, Catalog::builtin(), &printed).expect("runs again");
    assert_eq!(again.project, applied.project, "{printed}");
    // Taking one out sends what fed it where it went.
    let removed = apply_script(&applied.project, Catalog::builtin(), "bus remove music")
        .expect("runs")
        .project;
    assert_eq!(
        removed.sequences[0]
            .track("A1")
            .expect("the track")
            .bus
            .as_ref()
            .map(|b| b.as_str()),
        Some("all")
    );
    for bad in [
        "bus add x to=nowhere",
        "bus set nowhere gain=-3",
        "track set A1 bus=nowhere",
        "track set V1 bus=x",
        "bus add x\nbus add y to=x\nbus set x to=y",
        "bus add x\nbus fx x add color.basic@1",
        "bus add x\nbus fx x set 4 enabled=false",
    ] {
        assert!(
            apply_script(&project, Catalog::builtin(), bad).is_err(),
            "{bad}"
        );
    }
}
