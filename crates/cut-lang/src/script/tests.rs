use super::*;
use crate::doc::{parse_document, print_document, PrintOptions};
use cut_model::Value;

const BASE: &str = r#"cut 1
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
    let expected = r#"cut 1
project name=""

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:04:12 link=L1
    clip c2 media=m1 src=00:01:00:12 at=01:00:04:12 dur=00:00:02:12 link=L2
      set opacity={0f:0 12f:1:ease}
      fx color.basic exposure=0.5
    transition t1 from=c1 to=c2 fx=dissolve dur=00:00:00:12
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
        cut_model::Param::Static(Value::Number(2.0))
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
