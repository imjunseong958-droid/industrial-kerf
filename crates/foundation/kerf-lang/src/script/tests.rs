use super::*;
use crate::doc::parse_document;

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
