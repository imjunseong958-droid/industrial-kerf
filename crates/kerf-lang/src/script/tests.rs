use super::*;
use crate::doc::{parse_document, print_document, PrintOptions};
use kerf_model::Value;

const BASE: &str = r#"kerf 1
project name="t"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip c2 media=m1 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00 link=L2
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:00 link=L1
    clip a2 media=m1 src=00:01:00:00 at=01:00:04:00 dur=00:00:03:00 link=L2
"#;

fn base() -> Project {
    parse_document(BASE, Catalog::builtin()).unwrap().project
}

fn run(project: &Project, script: &str) -> Applied {
    apply_script(project, Catalog::builtin(), script).unwrap_or_else(|errors| panic!("{errors:#?}"))
}

fn fails(project: &Project, script: &str) -> Diagnostic {
    apply_script(project, Catalog::builtin(), script)
        .expect_err("expected the script to fail")
        .remove(0)
}

fn frames(time: Time) -> i64 {
    Rate::FPS_24.frames_exact(time).unwrap()
}

#[test]
fn builds_a_project_from_nothing() {
    let applied = run(
        &Project::default(),
        r#"
media add m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000
sequence add s1 name="Main" rate=24 size=1920x1080 start=01:00:00:00
track add V1 video
track add A1 audio
insert media=m1 src=00:00:10:00 dur=00:00:04:00 at=01:00:00:00 on=V1,A1 as=c1,a1
insert media=m1 src=00:01:00:00 out=00:01:03:00 at=01:00:04:00 on=V1,A1 as=c2,a2
roll c1 +12f
set c2 opacity={0f:0 12f:1:ease}
fx c2 add color.basic exposure=0.5
transition add as=t1 from=c1 to=c2 fx=dissolve dur=12f
marker add as=mk1 at=01:00:02:00 name="beat" color=green
"#,
    );
    let expected = r#"kerf 1
project name=""

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:12 link=L1
    clip c2 media=m1 src=00:01:00:12 at=01:00:04:12 dur=00:00:02:12 link=L2
      set opacity={0f:0 12f:1:ease}
      fx color.basic@1 exposure=0.5
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=00:00:00:12
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:12 link=L1
    clip a2 media=m1 src=00:01:00:12 at=01:00:04:12 dur=00:00:02:12 link=L2
  marker mk1 at=01:00:02:00 name="beat" color=green
"#;
    assert_eq!(
        print_document(
            &applied.project,
            Catalog::builtin(),
            PrintOptions::default()
        ),
        expected
    );
    assert_eq!(applied.report.created.len(), 4);
}

#[test]
fn errors_name_the_failing_line_and_change_nothing() {
    let project = base();
    let error = fails(&project, "set c1 opacity=0.5\n\nroll c2 +6f\n");
    assert_eq!((error.code, error.span.unwrap().line), ("roll.no_cut", 3));
    assert_eq!(project, base());

    let error = fails(&project, "zoom c1");
    assert!(error.message.contains("unknown command `zoom`"));
    let error = fails(&project, "trim c1 sideways -1f");
    assert_eq!(error.span.unwrap().column, 9);
}

#[test]
fn insert_gives_each_target_the_properties_of_its_kind() {
    let applied = run(
        &base(),
        "insert media=m1 src=0f dur=24f at=01:00:07:00 on=V1,A1 as=c3,a3 opacity=0.5 gain=-6",
    );
    let video = applied.project.clip("c3").unwrap();
    let audio = applied.project.clip("a3").unwrap();
    assert_eq!(video.props.keys().collect::<Vec<_>>(), ["opacity"]);
    assert_eq!(audio.props.keys().collect::<Vec<_>>(), ["gain"]);
    assert_eq!(video.link, audio.link);

    let error = fails(
        &base(),
        "insert media=m1 dur=24f at=01:00:07:00 on=V1 gain=-6",
    );
    assert!(error.message.contains("no target track takes `gain`"));
}

#[test]
fn effects_are_addressed_by_id_or_position() {
    let error = fails(
        &base(),
        "fx c1 add blur.gaussian\nfx c1 add blur.gaussian radius=8\nfx c1 set blur.gaussian radius=2",
    );
    assert_eq!(error.span.unwrap().line, 3);
    assert!(error.message.contains("name one by position (1, 2)"));

    let applied = run(
        &base(),
        "fx c1 add blur.gaussian\nfx c1 add blur.gaussian radius=8\nfx c1 set 2 radius=2\nfx c1 move 2 to=1",
    );
    let clip = applied.project.clip("c1").unwrap();
    assert_eq!(
        clip.effects[0].params["radius"],
        kerf_model::Param::Static(Value::Number(2.0))
    );

    // A version may be written when adding or naming one; an add without
    // one takes this Kerf's, and the journal says which.
    let applied = run(
        &base(),
        "fx c1 add blur.gaussian@1 radius=8\nfx c1 set blur.gaussian@1 radius=3",
    );
    let effect = &applied.project.clip("c1").unwrap().effects[0];
    assert_eq!((effect.fx.as_str(), effect.version), ("blur.gaussian", 1));
    let applied = run(&base(), "fx c1 add blur.gaussian radius=8");
    let printed = print_ops(&base(), Catalog::builtin(), &applied.ops);
    assert!(
        printed.contains("fx c1 add blur.gaussian@1 radius=8"),
        "{printed}"
    );
    let error = fails(&base(), "fx c1 add blur.gaussian@2");
    assert!(error.message.contains("newer than this Kerf"), "{error}");
    let error = fails(
        &base(),
        "fx c1 add blur.gaussian\nfx c1 set blur.gaussian@2 radius=3",
    );
    assert!(
        error.message.contains("no `blur.gaussian@2` effect"),
        "{error}"
    );
}

#[test]
fn absolute_forms_resolve_against_the_current_clip() {
    let applied = run(
        &base(),
        "trim c1 out to=01:00:03:12\nmove c2 at=01:00:10:00",
    );
    let project = &applied.project;
    assert_eq!(frames(project.clip("c1").unwrap().duration), 84);
    assert_eq!(frames(project.clip("a1").unwrap().duration), 84);
    assert_eq!(frames(project.clip("c2").unwrap().start), 240);
    assert_eq!(frames(project.clip("a2").unwrap().start), 240);
}

#[test]
fn several_sequences_need_a_use_line() {
    let applied = run(
        &base(),
        "sequence add s2 rate=24 size=1280x720\ntrack add V1 video",
    );
    assert_eq!(applied.project.sequence("s2").unwrap().tracks.len(), 1);
    let error = fails(&applied.project, "blade at=01:00:01:00");
    assert!(error.message.contains("use <id>"));
    run(&applied.project, "use s1\nblade at=01:00:01:00");
}

#[test]
fn printed_operations_replay_to_the_same_project() {
    let project = base();
    let applied = run(
        &project,
        r#"
insert media=m1 src=00:02:00:00 dur=36f at=01:00:02:00 on=V1,A1 as=c3,a3 opacity=0.8 name="cutaway"
slip c3 +6f
split c2 at=01:00:08:00 as=c2b
fx c3 add color.basic saturation=1.2
set c3 scale={0f:1 24f:1.2:ease} reset=opacity
transition add from=c1 to=c3 fx=dip dur=6f align=start color=#ffffff
marker add at=01:00:01:00 dur=12f name="note" note="check \"sync\""
extract from=01:00:09:00 to=01:00:09:12
track add V2 video before=V1 name="Overlay"
overwrite gen=title text="Day One" dur=48f at=01:00:00:00 on=V2 y=0.3
"#,
    );
    let printed = print_ops(&project, Catalog::builtin(), &applied.ops);
    let replayed = run(&project, &printed);
    assert_eq!(replayed.project, applied.project, "{printed}");
    assert!(!printed.contains("use "), "{printed}");
}

#[test]
fn masks_are_added_changed_and_removed() {
    let project = base();
    let applied = run(
        &project,
        r#"
mask c1 add k1 rect=0.25,0.25,0.75,0.75 feather=4
mask c1 add ring ellipse=0.5,0.5,0.3,0.2 mode=none at=1
mask c1 add line points="0,0 1,1" closed=false mode=none
fx c1 add generate.audiowaveform path=ring
mask c1 set k1 mode=subtract inverted=true opacity={0f:0 12f:1} reset=feather
mask c1 remove line
"#,
    );
    let clip = applied.project.clip("c1").unwrap();
    let names: Vec<&str> = clip.masks.iter().map(|mask| mask.name.as_str()).collect();
    assert_eq!(names, ["ring", "k1"]);
    let k1 = clip.mask("k1").unwrap();
    assert_eq!(k1.mode, kerf_model::MaskMode::Subtract);
    assert!(k1.inverted && !k1.params.contains_key("feather"));
    // The journal's script replays the same masks.
    let printed = print_ops(&project, Catalog::builtin(), &applied.ops);
    assert_eq!(
        run(&project, &printed).project,
        applied.project,
        "{printed}"
    );
    // A mask an effect reads stays; masks belong to pictures.
    let error = fails(&applied.project, "mask c1 remove ring");
    assert!(error.message.contains("not a mask of clip"), "{error:?}");
    let error = fails(&project, "mask a1 add k1 rect=0,0,1,1");
    assert!(
        error.message.contains("only pictures carry masks"),
        "{error:?}"
    );
    let error = fails(&project, "mask c1 add k1 rect=0.5,0,0.25,1");
    assert!(error.message.contains("right past its left"), "{error:?}");
    let error = fails(&project, "mask c1 add k1 rect=0,0,1,1 points=\"0,0 1,1\"");
    assert!(error.message.contains("one outline"), "{error:?}");
}

#[test]
fn print_op_switches_sequences_explicitly() {
    let two = run(
        &base(),
        "sequence add s2 rate=25 size=1280x720\nuse s2\ntrack add V1 video",
    )
    .project;
    let op = Op::Blade {
        sequence: Id::new("s2").unwrap(),
        at: Rate::FPS_25.time_of(25),
        tracks: None,
    };
    assert_eq!(
        print_op(&two, Catalog::builtin(), &op),
        "use s2\nblade at=00:00:01:00"
    );
}

#[test]
fn positions_can_anchor_to_the_end_and_to_clips() {
    let project = base();
    // Append at the end, then insert just after c1, both without timecode.
    let applied = run(
        &project,
        "insert media=m1 src=00:02:00:00 dur=24f at=end on=V1,A1 as=c3,a3
insert media=m1 src=00:03:00:00 dur=12f at=c1.end on=V1,A1 as=c4,a4
marker add at=c4.start+6f as=mk1",
    );
    let sequence = &applied.project.sequences[0];
    let clock = Clock::of_sequence(sequence);
    let at = |id: &str| clock.print_position(sequence.tracks[0].clip(id).unwrap().start);
    assert_eq!(at("c3"), "01:00:07:12", "after c2, moved by c4's insert");
    assert_eq!(at("c4"), "01:00:04:00");
    assert_eq!(at("c2"), "01:00:04:12");
    assert_eq!(clock.print_position(sequence.markers[0].at), "01:00:04:06");

    let lifted = run(&project, "lift from=c1.start+1s to=c1.end-1s");
    assert_eq!(lifted.project.sequences[0].tracks[0].clips.len(), 3);

    assert!(fails(&project, "split c1 at=c9.end")
        .message
        .contains("no clip `c9`"));
    assert!(fails(&project, "split c1 at=c1.end6f")
        .message
        .contains("amount"));
}

#[test]
fn trim_to_a_source_position() {
    let project = base();
    // c1 plays m1 from 00:00:10:00; end it on 00:00:12:12 of the media.
    let trimmed = run(&project, "trim c1 out src=00:00:12:12 ripple");
    let clip = trimmed.project.sequences[0].tracks[0].clip("c1").unwrap();
    assert_eq!(frames(clip.duration), 60);
    // At double speed, a second of source is half a second of sequence.
    let fast = run(&project, "speed c1 2\ntrim c1 in src=00:00:11:00");
    let clip = fast.project.sequences[0].tracks[0].clip("c1").unwrap();
    assert_eq!(
        clip.source_range().unwrap().0,
        Time::from_seconds(11.0).unwrap()
    );
    assert!(fails(&project, "trim c1 out src=12s to=01:00:01:00")
        .message
        .contains("not both"));
}
