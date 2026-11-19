use super::*;
use crate::doc::{parse_document, print_document, PrintOptions};
use crate::{Breaks, Speech, Word};
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

    // A track that is not there takes nothing: the line is refused for the
    // track, with the tracks there are and the line that adds it, whatever
    // else it sets.
    let error = fails(
        &base(),
        "overwrite media=m1 dur=24f at=01:00:07:00 on=V2 as=c9 y=120",
    );
    assert_eq!(error.code, "ref.missing", "{error:?}");
    assert!(
        error.message.contains(
            "sequence `s1` has no track `V2` (it has V1, A1); `track add V2 video` adds one"
        ),
        "{}",
        error.message
    );
    assert_eq!(error.span.map(|span| span.line), Some(1), "{error:?}");

    // What `set` writes on a clip that is there is said to be that, with
    // the line to write.
    let error = fails(
        &base(),
        "overwrite media=m1 dur=24f at=01:00:07:00 on=V1 as=c9 parent=c1",
    );
    assert!(
        error.message.contains(
            "`parent=` is set on a clip once it is placed, not where it is placed: \
             after this line, `set c9 parent=c1`"
        ),
        "{}",
        error.message
    );
    let error = fails(
        &base(),
        "insert media=m1 dur=24f at=01:00:07:00 on=V1 enabled=false",
    );
    assert!(
        error
            .message
            .contains("after this line, `set <clip> enabled=false`"),
        "{}",
        error.message
    );
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
fn track_roles_and_sync_locks_set_print_and_replay() {
    let project = base();
    let applied = run(
        &project,
        "track add V2 video role=Titles sync=off\ntrack set A1 name=\"Dialogue\" role=\"Room tone\"\ntrack set V1 sync=off",
    );
    let printed = print_ops(&project, Catalog::builtin(), &applied.ops);
    assert_eq!(
        printed,
        "track add V2 video\n\
         track set V2 role=titles sync=off\n\
         track set A1 name=\"Dialogue\"\n\
         track set A1 role=\"Room tone\"\n\
         track set V1 sync=off\n"
    );
    let replayed = run(&project, &printed);
    assert_eq!(replayed.project, applied.project);
    let tracks = &applied.project.sequences[0].tracks;
    assert_eq!(
        tracks
            .iter()
            .map(|t| (t.id.as_str(), t.role.as_deref(), t.sync_locked))
            .collect::<Vec<_>>(),
        [
            ("V1", None, false),
            ("A1", Some("Room tone"), true),
            ("V2", Some("titles"), false)
        ]
    );

    // An empty role clears it.
    let cleared = run(&applied.project, "track set A1 role=\"\" sync=on");
    let printed = print_ops(&applied.project, Catalog::builtin(), &cleared.ops);
    assert_eq!(printed, "track set A1 role=\"\" sync=on\n");
    assert_eq!(cleared.project.sequences[0].tracks[1].role, None);
    assert!(fails(&project, "track set V1 sync=sometimes")
        .message
        .contains("on or off"));
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
        .contains("no clip or marker `c9`"));
    assert!(fails(&project, "split c1 at=c1.end6f")
        .message
        .contains("amount"));
}

#[test]
fn positions_can_name_markers() {
    let project = run(
        &base(),
        "marker add as=beat1 at=01:00:01:00
marker add as=chorus at=01:00:05:00 dur=12f
marker add as=end-credits at=01:00:06:00
marker add as=end at=01:00:02:00",
    )
    .project;
    let applied = run(
        &project,
        "split c1 at=beat1 as=c1b
split c2 at=chorus.end as=c2b
split c2b at=end-credits as=c2c
marker add at=beat1+2f as=mk1
marker add at=end-1f as=mk2",
    );
    let sequence = &applied.project.sequences[0];
    let start = |id: &str| frames(sequence.tracks[0].clip(id).unwrap().start);
    assert_eq!(start("c1b"), 24);
    assert_eq!(start("c2b"), 132, "where the marker's duration ends");
    assert_eq!(start("c2c"), 144, "`-` is part of the id, not an amount");
    let at = |id: &str| frames(sequence.marker(id).unwrap().at);
    assert_eq!(at("mk1"), 26);
    assert_eq!(
        at("mk2"),
        167,
        "`end` is the sequence's end, not the marker"
    );

    // A marker named like the start of a longer id does not take its anchors.
    let prefixed = run(
        &project,
        "marker add as=verse at=01:00:00:12
marker add as=verse-2 at=01:00:01:12 dur=6f
split c1 at=c1.start+2s as=intro-b
marker add as=intro at=01:00:00:06
split intro-b at=intro-b.end-12f as=c1c
marker add at=verse-2.end as=mk3",
    );
    let sequence = &prefixed.project.sequences[0];
    assert_eq!(frames(sequence.tracks[0].clip("c1c").unwrap().start), 84);
    assert_eq!(frames(sequence.marker("mk3").unwrap().at), 42);

    assert!(fails(&project, "split c1 at=nowhere")
        .message
        .contains("not a time"));
    assert!(fails(&project, "split c1 at=nowhere.end")
        .message
        .contains("no clip or marker `nowhere`"));
}

#[test]
fn positions_can_name_where_a_clip_plays_a_source_position() {
    let project = base();
    // c1 plays m1 from 00:00:10:00 at the start; c2 plays 00:01:00:00 at 4s.
    let applied = run(
        &project,
        "split c1 at=c1@00:00:11:12 as=c1b
marker add at=c2@00:01:01:00+2f as=mk1",
    );
    let sequence = &applied.project.sequences[0];
    assert_eq!(frames(sequence.tracks[0].clip("c1b").unwrap().start), 36);
    assert_eq!(frames(sequence.marker("mk1").unwrap().at), 122);
    // At double speed two seconds of source pass in one; in reverse the
    // clip starts on its source out.
    let fast = run(&project, "speed c1 2\nsplit c1 at=c1@00:00:12:00 as=c1b");
    assert_eq!(frames(fast.project.clip("c1b").unwrap().start), 24);
    let reverse = run(&project, "speed c1 -1\nsplit c1 at=c1@00:00:13:00 as=c1b");
    assert_eq!(frames(reverse.project.clip("c1b").unwrap().start), 24);

    let outside = fails(&project, "split c1 at=c1@00:00:20:00");
    assert!(
        outside
            .message
            .contains("plays 00:00:10:00 to 00:00:14:00, not `00:00:20:00`"),
        "{outside:?}"
    );
    assert!(fails(&project, "split c1 at=c9@00:00:11:00")
        .message
        .contains("no clip `c9`"));
}

#[test]
fn a_line_that_names_what_is_not_there_is_not_a_syntax_error() {
    // The line is written as it should be: what it names is missing, as
    // the model says of an operation that names it.
    for (script, says) in [
        ("trim c9 in +1f", "no clip `c9`"),
        ("use s9", "no sequence `s9`"),
        ("transition set t9 dur=6f", "no transition `t9`"),
        ("marker remove m9", "no marker `m9`"),
    ] {
        let error = fails(&base(), script);
        assert!(error.message.contains(says), "{script}: {}", error.message);
        assert_eq!(error.code, "ref.missing", "{script}: {}", error.message);
    }
    // A line that cannot be read is still one.
    assert_eq!(fails(&base(), "trim c1 sideways +1f").code, "syntax");
}

/// What analysis knows of m1: a few words, a pause, the gap "So we" part
/// for, and a cut.
struct Known;

impl Facts for Known {
    fn speech(&self, _: &Media) -> Speech {
        let word = |start: f64, end: f64, text: &str| Word {
            start,
            end,
            text: text.into(),
        };
        Speech {
            words: vec![
                word(10.5, 11.0, " So"),
                word(11.0, 11.4, " we"),
                word(11.4, 12.0, " closed,"),
                word(12.6, 13.0, " the"),
                word(13.0, 13.5, " deal."),
                word(20.0, 20.5, " So"),
            ],
            covered: vec![(0.0, 600.0)],
        }
    }

    fn breaks(&self, _: &Media) -> Option<Breaks> {
        Some(Breaks {
            pauses: vec![(12.0, 12.6)],
            gaps: vec![(10.95, 11.07)],
            cuts: vec![11.5],
        })
    }
}

fn known(project: &Project, script: &str) -> Result<Applied, Diagnostic> {
    apply_script_with(project, Catalog::builtin(), &Known, script).map_err(|mut e| e.remove(0))
}

fn source(applied: &Applied, clip: &str) -> (i64, i64) {
    let (from, to) = applied.project.clip(clip).unwrap().source_range().unwrap();
    (frames(from), frames(to))
}

#[test]
fn src_and_out_can_quote_what_was_said() {
    let project = base();
    // From the frame "So we" starts to the end of the frame "the deal" ends.
    let placed = known(
        &project,
        r#"insert media=m1 src="so we" out="The deal." at=end on=V1,A1 as=c3,a3"#,
    )
    .unwrap();
    assert_eq!(source(&placed, "c3"), (252, 324));
    // A trim takes a phrase's start for an in point and its end for an out.
    let trimmed = known(
        &project,
        "trim c1 out src=\"closed\"\ntrim c1 in src=\"we\"",
    )
    .unwrap();
    assert_eq!(source(&trimmed, "c1"), (264, 288));

    let twice = known(&project, r#"trim c1 in src="so""#).unwrap_err();
    assert!(
        twice
            .message
            .contains("is said 2 times in `m1`, at 00:00:10:12, 00:00:20:00"),
        "{twice:?}"
    );
    let unheard = known(&project, r#"trim c1 in src="nowhere""#).unwrap_err();
    assert!(unheard.message.contains("heard \"nowhere\""), "{unheard:?}");
    // Without transcripts there is nothing to quote.
    assert!(fails(&project, r#"trim c1 in src="so we""#)
        .message
        .contains("no transcript of `m1`"));
}

#[test]
fn edges_snap_into_pauses_and_onto_cuts() {
    let project = base();
    // c1 plays m1 from 10s at the sequence's start. 1s 21f plays 11.875s,
    // just before the pause that starts at 12s: the out point moves into it,
    // to the frame nearest 12.15s, which leaves the word before it its decay.
    let trimmed = known(&project, "trim c1 out to=01:00:01:21 snap=pause").unwrap();
    assert_eq!(frames(trimmed.project.clip("c1").unwrap().duration), 52);
    // What moved is said, with the words it is now between: the document
    // shows where an edge is, and not that it was asked for elsewhere.
    assert_eq!(
        trimmed.snapped,
        [
            "the out of `c1` went 7 frames later, to 01:00:02:04: into the 0.60s pause \
          of `m1` at 00:00:12:00, between \"closed,\" and \"the\""
        ]
    );
    // An in point from early in "the", which follows the pause at 12.6s,
    // stops a tenth of a second short of it.
    let trimmed = known(&project, "trim c1 in to=01:00:02:18 snap=pause").unwrap();
    assert_eq!(source(&trimmed, "c1").0, 300);
    assert_eq!(
        trimmed.snapped,
        [
            "the in of `c1` went 6 frames earlier, to 01:00:02:12: into the 0.60s pause \
          of `m1` at 00:00:12:00, between \"closed,\" and \"the\""
        ]
    );
    // 1s 10f plays 11.42s; the picture cuts at 11.5s.
    let split = known(&project, "split c1 at=01:00:01:10 snap=cut as=c1b").unwrap();
    assert_eq!(frames(split.project.clip("c1b").unwrap().start), 36);
    assert_eq!(
        split.snapped,
        [
            "the split of `c1` went 2 frames later, to 01:00:01:12: onto the cut of `m1` \
          at 00:00:11:12"
        ]
    );
    // A placed source's in and out points move into the pause too, and one
    // already in place is not said.
    let placed = known(
        &project,
        "insert media=m1 src=00:00:11:20 out=00:00:12:14 snap=pause at=end on=V1 as=c3",
    )
    .unwrap();
    assert_eq!(source(&placed, "c3"), (292, 300));
    assert_eq!(placed.snapped.len(), 2, "{:?}", placed.snapped);
    assert!(placed.snapped[0].starts_with("src= went 8 frames later, to 00:00:12:04:"));
    assert!(placed.snapped[1].starts_with("out= went 2 frames earlier, to 00:00:12:12:"));
    let stays = known(&project, "trim c1 out to=01:00:02:06 snap=pause").unwrap();
    assert!(stays.snapped.is_empty(), "{:?}", stays.snapped);

    let far = known(&project, "split c1 at=01:00:00:12 snap=cut within=2f").unwrap_err();
    assert!(far.message.contains("no cut of `m1` within"), "{far:?}");
    assert!(fails(&project, "split c1 at=01:00:01:10 snap=cut")
        .message
        .contains("not indexed yet"));
    assert!(fails(&project, "split c1 at=01:00:01:10 within=2f")
        .message
        .contains("within= goes with snap="));
    assert!(fails(&project, "split c1 at=01:00:01:10 snap=beat")
        .message
        .contains("use pause or cut"));
}

#[test]
fn an_edge_snaps_past_no_word() {
    let project = base();
    // 2s 20f plays 12.83s, past the middle of "the": the pause before it is
    // on the other side of the word, and "the deal." has none after it.
    let across = known(&project, "trim c1 in to=01:00:02:20 snap=pause").unwrap_err();
    assert_eq!(
        across.message,
        "no pause of `m1` within 0.50s of 00:00:12:20 would do here: the pause at 00:00:12:00 \
         is on the other side of \"the\""
    );
    // An out point asked for where "the" ends is the same.
    let across = known(
        &project,
        "insert media=m1 src=00:00:11:20 out=00:00:13:00 snap=pause at=end on=V1 as=c3",
    )
    .unwrap_err();
    assert!(
        across.message.contains("is on the other side of \"the\""),
        "{across:?}"
    );
    // "So we" is said in one breath, parting for an eighth of a second: by
    // the transcript that is between two words, so a split asked for where
    // "we" starts goes into it and not on to the pause past "we closed,".
    let split = known(&project, "split c1 at=01:00:01:01 snap=pause as=c1b").unwrap();
    assert_eq!(frames(split.project.clip("c1b").unwrap().start), 24);
    assert_eq!(
        split.snapped,
        [
            "the split of `c1` went 1 frame earlier, to 01:00:01:00: into the 0.12s gap \
          of `m1` at 00:00:10:23, where \"So\" and \"we\" run together"
        ]
    );
    // From inside "we" the gap is the nearer end of the word, and the pause
    // past "closed," is never reached.
    let split = known(&project, "split c1 at=01:00:01:03 snap=pause as=c1b").unwrap();
    assert_eq!(frames(split.project.clip("c1b").unwrap().start), 24);
    // Facts with no transcript do not say what a gap is.
    struct Unheard;
    impl Facts for Unheard {
        fn speech(&self, _: &Media) -> Speech {
            Speech::default()
        }
        fn breaks(&self, media: &Media) -> Option<Breaks> {
            Known.breaks(media)
        }
    }
    let unheard = apply_script_with(
        &project,
        Catalog::builtin(),
        &Unheard,
        "split c1 at=01:00:01:01 snap=pause as=c1b",
    )
    .unwrap_err()
    .remove(0);
    assert_eq!(
        unheard.message,
        "no pause of `m1` within 0.50s of 00:00:11:01 would do here: the sound parts for 0.12s \
         at 00:00:10:23, which no transcript covers to say is between two words: look at `m1` \
         with transcript there first"
    );
}

/// A recording of m1 as an index and a transcriber leave it: "So", a pause,
/// "um,", a longer one, "we closed." in one breath, and a wait before
/// "Then", which c1 stops short of. `wait` is the level of something that
/// sounds in the pause after "um," and has no word.
struct Talk {
    wait: Option<f32>,
}

impl Talk {
    fn sound(&self) -> crate::Sound {
        let mut levels = vec![-55.0f32; 1500];
        // Talk before any of it, so that the levels tell speech from the room.
        for (from, to) in [(100, 900), (1050, 1090), (1130, 1160), (1240, 1320), (1450, 1490)] {
            levels[from..to].fill(-20.0);
        }
        if let Some(level) = self.wait {
            levels[1190..1210].fill(level);
        }
        crate::Sound::new(0.01, levels)
    }
}

impl Facts for Talk {
    fn speech(&self, _: &Media) -> Speech {
        let word = |start: f64, end: f64, text: &str| Word {
            start,
            end,
            text: text.into(),
        };
        Speech {
            words: vec![
                word(10.5, 10.9, " So"),
                word(11.3, 11.6, " um,"),
                word(12.4, 12.7, " we"),
                word(12.7, 13.2, " closed."),
                word(14.5, 14.9, " Then"),
            ],
            covered: vec![(0.0, 15.0)],
        }
    }

    fn breaks(&self, _: &Media) -> Option<Breaks> {
        let sound = self.sound();
        Some(Breaks {
            pauses: sound.pauses(),
            gaps: sound.gaps(),
            cuts: Vec::new(),
        })
    }

    fn sound(&self, _: &Media) -> Option<crate::Sound> {
        Some(self.sound())
    }
}

fn talk(project: &Project, script: &str) -> Result<Applied, Diagnostic> {
    apply_script_with(project, Catalog::builtin(), &Talk { wait: None }, script)
        .map_err(|mut e| e.remove(0))
}

#[test]
fn words_come_out_of_a_clip_by_one_line() {
    let project = base();
    // c1 plays m1 from 10s. "um," goes with the quiet round it, down to the
    // pause an edge's air makes: 0.15s after "So" and 0.1s before "we".
    let cut = talk(&project, r#"extract c1 said="um""#).unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &cut.ops).trim(),
        "extract from=01:00:01:01 to=01:00:02:07"
    );
    assert_eq!(frames(cut.project.clip("c2").unwrap().start), 66);
    assert_eq!(
        cut.snapped,
        [
            "\"um\" goes from `c1`, 01:00:01:01 to 01:00:02:07, which leaves 0.25s between \
             \"So\" and \"we\""
        ]
    );
    // A longer pause is longer before the cut, as far as the quiet after
    // "So" goes, and the voice comes in as soon after it.
    let slower = talk(&project, r#"extract c1 said="um" leave=0.5s"#).unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &slower.ops).trim(),
        "extract from=01:00:01:06 to=01:00:02:07"
    );
    assert!(
        slower.snapped[0].contains("which leaves 0.46s"),
        "{:?}",
        slower.snapped
    );
    // Words that end the clip go to its end, and lifted they leave a gap.
    let last = talk(&project, r#"extract c1 said="we closed""#).unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &last.ops).trim(),
        "extract from=01:00:01:18 to=01:00:04:00"
    );
    assert_eq!(
        last.snapped,
        ["\"we closed\" goes from `c1`, 01:00:01:18 to 01:00:04:00, which leaves 0.15s"]
    );
    let lifted = talk(&project, r#"lift c1 said="um""#).unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &lifted.ops).trim(),
        "lift from=01:00:01:01 to=01:00:02:07"
    );
    assert_eq!(frames(lifted.project.clip("c2").unwrap().start), 96);

    // "we" runs on into "closed.": there is no quiet to cut in after it.
    let run_on = talk(&project, r#"extract c1 said="we""#).unwrap_err();
    assert_eq!(
        run_on.message,
        "nowhere to cut after \"we\": no pause of `m1` within 0.50s of 00:00:12:17 would do \
         here: the pause at 00:00:11:14 is on the other side of \"we\""
    );
    // Words are the clip's own, and a transcript's.
    let elsewhere = talk(&project, r#"extract c1 said="then""#).unwrap_err();
    assert_eq!(
        elsewhere.message,
        "`c1` does not play \"then\", which `m1` has at 00:00:14:12"
    );
    assert!(talk(&project, r#"extract c1 said="nowhere""#)
        .unwrap_err()
        .message
        .contains("no transcript of `m1` heard \"nowhere\""));
}

#[test]
fn a_pause_comes_down_to_a_length() {
    let project = base();
    // The 0.8s after "um," comes down to a quarter of a second.
    let cut = talk(&project, r#"extract c1 pause="um""#).unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &cut.ops).trim(),
        "extract from=01:00:01:18 to=01:00:02:07"
    );
    assert_eq!(
        cut.snapped,
        [
            "the pause after \"um\" in `c1`, 0.80s, comes down to 0.26s between \"um,\" and \
             \"we\": 01:00:01:18 to 01:00:02:07 goes"
        ]
    );
    // One no longer than is asked for is not shortened.
    let long = talk(&project, r#"extract c1 pause="um" leave=1s"#).unwrap_err();
    assert_eq!(
        long.message,
        "the pause after \"um\" is 0.80s, which leave=1.00s does not shorten"
    );
    // A breath in the pause goes with it.
    let breath = Talk { wait: Some(-40.0) };
    let cut = apply_script_with(
        &project,
        Catalog::builtin(),
        &breath,
        r#"extract c1 pause="um""#,
    )
    .unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &cut.ops).trim(),
        "extract from=01:00:01:18 to=01:00:02:07"
    );
    // What sounds as loud as speech there and has no word may be one a
    // transcriber missed: it is not taken for a pause.
    let missed = Talk { wait: Some(-20.0) };
    let refused = apply_script_with(
        &project,
        Catalog::builtin(),
        &missed,
        r#"extract c1 pause="um""#,
    )
    .unwrap_err()
    .remove(0);
    assert_eq!(
        refused.message,
        "the wait after \"um\" holds sound at the level of speech, from 00:00:11:22 to \
         00:00:12:02 of `m1`, which no word accounts for: listen to it, and take it out by \
         from= and to= if it is to go"
    );
    // A lift closes nothing up.
    assert!(talk(&project, r#"lift c1 pause="um""#)
        .unwrap_err()
        .message
        .contains("pause= closes a pause up: use extract"));
}

#[test]
fn a_range_snaps_by_the_clip_it_names() {
    let project = base();
    // From late in the pause before "um," and just into "we": each edge
    // goes into the pause beside it.
    let cut = talk(
        &project,
        "extract c1 from=01:00:01:07 to=01:00:02:10 snap=pause",
    )
    .unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &cut.ops).trim(),
        "extract from=01:00:01:05 to=01:00:02:07"
    );
    assert_eq!(frames(cut.project.clip("c2").unwrap().start), 70);
    assert_eq!(
        cut.snapped,
        [
            "from= went 2 frames earlier, to 01:00:01:05: into the 0.40s pause of `m1` at \
             00:00:10:22, between \"So\" and \"um,\"",
            "to= went 3 frames earlier, to 01:00:02:07: into the 0.80s pause of `m1` at \
             00:00:11:14, between \"um,\" and \"we\""
        ]
    );
    for (script, says) in [
        (
            "extract from=01:00:01:07 to=01:00:02:10 snap=pause",
            "snap= goes by a clip's sound and words: name the clip",
        ),
        (
            "extract c1 from=01:00:01:07 to=01:00:02:10",
            "naming `c1` puts the edges by its sound: add snap=pause or snap=cut",
        ),
        (
            "extract c1 from=01:00:01:07 to=01:00:09:00 snap=pause",
            "to= is not in `c1`, which plays from 01:00:00:00 to 01:00:04:00",
        ),
        (
            r#"extract c1 said="um" from=01:00:01:07 to=01:00:02:10"#,
            "with a clip, give said=",
        ),
        (
            "extract c1 from=01:00:01:07 to=01:00:02:10 snap=pause leave=1s",
            "leave= goes with said= or pause=",
        ),
        (r#"extract c1 c2 said="um""#, "unexpected argument"),
    ] {
        let error = talk(&project, script).unwrap_err();
        assert!(error.message.contains(says), "{script}: {}", error.message);
    }
    // Without a clip a range is its two positions, as it always was.
    let plain = talk(&project, "extract from=01:00:01:07 to=01:00:02:10").unwrap();
    assert_eq!(frames(plain.project.clip("c2").unwrap().start), 69);
}

/// Where the blade a script ends with was put, in frames.
fn bladed(applied: &Applied) -> i64 {
    match applied.ops.last() {
        Some(Op::Blade { at, .. }) => frames(*at),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_blade_snaps_as_a_split_of_the_one_sound_it_cuts() {
    let project = base();
    // c1 and a1 play m1 together from 10s, so the blade has one sound to go
    // by. 1s 21f plays 11.875s, just before the pause at 12s: it goes where
    // a split of either goes, and cuts both there.
    let blade = known(&project, "blade at=01:00:01:21 snap=pause").unwrap();
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &blade.ops).trim(),
        "blade at=01:00:02:04"
    );
    assert_eq!(
        blade.snapped,
        [
            "the blade went 7 frames later, to 01:00:02:04: into the 0.60s pause of `m1` at \
             00:00:12:00, between \"closed,\" and \"the\""
        ]
    );
    for clip in ["c1", "a1"] {
        assert_eq!(frames(blade.project.clip(clip).unwrap().duration), 52);
    }
    let split = known(&project, "split c1 at=01:00:01:21 snap=pause").unwrap();
    assert_eq!(split.project, blade.project);
    // One asked for in the pause stays, and nothing is said of it.
    let stays = known(&project, "blade at=01:00:02:06 snap=pause").unwrap();
    assert_eq!(bladed(&stays), 54);
    assert!(stays.snapped.is_empty(), "{:?}", stays.snapped);
    // Between "So" and "we", which part for an eighth of a second.
    let gap = known(&project, "blade at=01:00:01:01 snap=pause").unwrap();
    assert_eq!(
        gap.snapped,
        [
            "the blade went 1 frame earlier, to 01:00:01:00: into the 0.12s gap of `m1` at \
             00:00:10:23, where \"So\" and \"we\" run together"
        ]
    );
    // 1s 10f plays 11.42s; the picture cuts at 11.5s.
    let cut = known(&project, "blade at=01:00:01:10 snap=cut").unwrap();
    assert_eq!(
        cut.snapped,
        [
            "the blade went 2 frames later, to 01:00:01:12: onto the cut of `m1` at \
             00:00:11:12"
        ]
    );
    // Past the middle of "the" the pause is on the other side of the word,
    // and the refusal names the clip that says it.
    let across = known(&project, "blade at=01:00:02:20 snap=pause").unwrap_err();
    assert_eq!(
        across.message,
        "in `a1`, no pause of `m1` within 0.50s of 00:00:12:20 would do here: the pause at \
         00:00:12:00 is on the other side of \"the\""
    );
    let far = known(&project, "blade at=01:00:00:12 snap=cut within=2f").unwrap_err();
    assert!(far.message.contains("no cut of `m1` within"), "{far:?}");
    // Past the last clip there is nothing to go by.
    let nothing = known(&project, "blade at=01:00:08:00 snap=pause").unwrap_err();
    assert_eq!(
        nothing.message,
        "nothing the blade cuts at 01:00:08:00 has a sound to snap to"
    );
    assert!(fails(&project, "blade at=01:00:01:10 snap=cut")
        .message
        .contains("not indexed yet"));
}

#[test]
fn a_blade_without_snap_is_where_it_was_asked_for() {
    let project = base();
    // In "closed,", three frames short of a pause: it is one operation at
    // the time it was given, and reads no facts.
    let plain = run(&project, "blade at=01:00:01:21");
    assert_eq!(
        plain.ops,
        [Op::Blade {
            sequence: Id::new("s1").unwrap(),
            at: Rate::FPS_24.time_of(45),
            tracks: None,
        }]
    );
    assert!(plain.snapped.is_empty());
    for clip in ["c1", "a1"] {
        assert_eq!(frames(plain.project.clip(clip).unwrap().duration), 45);
    }
    // With facts at hand it is the same.
    assert_eq!(known(&project, "blade at=01:00:01:21").unwrap(), plain);
    let one = run(&project, "blade at=01:00:01:21 tracks=A1");
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &one.ops).trim(),
        "blade at=01:00:01:21 tracks=A1"
    );
    assert_eq!(frames(one.project.clip("c1").unwrap().duration), 96);
    assert!(fails(&project, "blade at=01:00:01:21 within=2f")
        .message
        .contains("within= goes with snap="));
    assert!(fails(&project, "blade at=01:00:01:21 snap=beat")
        .message
        .contains("use pause or cut"));
}

/// m1 as [`Known`] has it, and m2, a second voice recorded beside it: "Good
/// morning." and "Fine.", with a pause before each, and a cut of its own
/// picture.
struct Two;

impl Facts for Two {
    fn speech(&self, media: &Media) -> Speech {
        if media.id.as_str() != "m2" {
            return Known.speech(media);
        }
        let word = |start: f64, end: f64, text: &str| Word {
            start,
            end,
            text: text.into(),
        };
        Speech {
            words: vec![
                word(30.85, 31.4, " Good"),
                word(31.4, 32.25, " morning."),
                word(32.9, 33.4, " Fine."),
            ],
            covered: vec![(0.0, 600.0)],
        }
    }

    fn breaks(&self, media: &Media) -> Option<Breaks> {
        if media.id.as_str() != "m2" {
            return Known.breaks(media);
        }
        Some(Breaks {
            pauses: vec![(30.25, 30.85), (32.25, 32.9)],
            gaps: Vec::new(),
            cuts: vec![31.25],
        })
    }
}

/// `script` on the base project with m2 laid over c1 and a1 from `src`, its
/// picture d1 on V2 and its sound b1 on A2.
fn two_from(src: &str, script: &str) -> Result<Applied, Diagnostic> {
    let script = format!(
        "media add m2 path=\"b.mov\" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000\n\
         track add V2 video\n\
         track add A2 audio\n\
         overwrite media=m2 src={src} dur=00:00:04:00 at=01:00:00:00 on=V2,A2 as=d1,b1\n\
         {script}"
    );
    apply_script_with(&base(), Catalog::builtin(), &Two, &script).map_err(|mut e| e.remove(0))
}

/// With m2 from 30s.
fn two(script: &str) -> Result<Applied, Diagnostic> {
    two_from("00:00:30:00", script)
}

#[test]
fn a_blade_goes_where_every_sound_it_cuts_is_quiet() {
    // At 1s 22f a1 is ending "closed,", before a pause it plays from 2s to
    // 2s 14f, and b1 is in "morning.", before one it plays from 2s 6f to
    // 2s 21f. A split of a1 goes to 2s 4f, where "morning." is still being
    // said; the blade goes on to where both are quiet, clear of that too.
    let blade = two("blade at=01:00:01:22 snap=pause").unwrap();
    assert_eq!(bladed(&blade), 58);
    assert_eq!(
        blade.snapped,
        [
            "the blade went 12 frames later, to 01:00:02:10: into the 0.60s pause of `m1` at \
             00:00:12:00, between \"closed,\" and \"the\", and the 0.65s pause of `m2` at \
             00:00:32:06, between \"morning.\" and \"Fine.\", quiet together for 0.35s"
        ]
    );
    // It is one cut: every clip under it ends on the same frame.
    for clip in ["c1", "a1", "d1", "b1"] {
        assert_eq!(frames(blade.project.clip(clip).unwrap().duration), 58);
    }
    // With tracks= only their clips are cut, and only their sound gone by.
    let one = two("blade at=01:00:01:22 tracks=V1,A1 snap=pause").unwrap();
    assert_eq!(bladed(&one), 52);
    assert_eq!(frames(one.project.clip("b1").unwrap().duration), 96);
    // m2 four frames later has its pause from 2s 10f, and the two are quiet
    // together for less than the air an edge leaves either side, 0.25s:
    // that is divided as a short pause of one is, and said.
    let short = two_from("00:00:29:20", "blade at=01:00:02:02 snap=pause").unwrap();
    assert_eq!(bladed(&short), 61);
    assert!(
        short.snapped[0].ends_with("quiet together for 0.18s"),
        "{:?}",
        short.snapped
    );

    // At 1s 1f a1 is between "So" and "we" and b1 is in "Good", with a pause
    // just before it: each has somewhere to go and it is not the same place,
    // so nothing moves, and each clip is named with the word in its way.
    let apart = two("blade at=01:00:01:01 snap=pause").unwrap_err();
    assert_eq!(
        apart.message,
        "the clips this blade cuts have no quiet in common within 0.50s of 01:00:01:01: `a1` \
         would take it to 01:00:01:00, into the 0.12s gap of `m1` at 00:00:10:23, where \"So\" \
         and \"we\" run together, while `b1` says \"Good\"; `b1` would take it to 01:00:00:18, \
         into the 0.60s pause of `m2` at 00:00:30:06, before \"Good\", while `a1` says \"So\": \
         cut them one at a time with tracks="
    );
    // One that is quiet where the blade was asked for stays for no other:
    // with m2 from 28s 20f, a1 is in its pause at 2s 4f and b1 in "Good".
    let stays = two_from("00:00:28:20", "blade at=01:00:02:04 snap=pause").unwrap_err();
    assert!(
        stays.message.contains(
            "`a1` is quiet there, in the 0.60s pause of `m1` at 00:00:12:00, between \
             \"closed,\" and \"the\", while `b1` says \"Good\"; `b1` would take it to 01:00:01:22,"
        ),
        "{stays:?}"
    );
    assert_eq!(bladed(&two("blade at=01:00:01:01 tracks=A1 snap=pause").unwrap()), 24);
    assert_eq!(bladed(&two("blade at=01:00:01:01 tracks=A2 snap=pause").unwrap()), 18);
    // A picture cut alone goes by its media's sound, as a split of it does.
    assert_eq!(bladed(&two("blade at=01:00:01:01 tracks=V2 snap=pause").unwrap()), 18);
    // One sound with nowhere to go refuses the blade too: past the middle
    // of "the", where b1 is in its pause.
    let across = two("blade at=01:00:02:20 snap=pause").unwrap_err();
    assert_eq!(
        across.message,
        "in `a1`, no pause of `m1` within 0.50s of 00:00:12:20 would do here: the pause at \
         00:00:12:00 is on the other side of \"the\""
    );

    // A cut is the nearest of either picture: m2's at 1s 6f, m1's at 1s 12f.
    let cut = two("blade at=01:00:01:07 snap=cut").unwrap();
    assert_eq!(
        cut.snapped,
        [
            "the blade went 1 frame earlier, to 01:00:01:06: onto the cut of `m2` at \
             00:00:31:06"
        ]
    );
    let cut = two("blade at=01:00:01:10 snap=cut").unwrap();
    assert_eq!(bladed(&cut), 36);
    assert!(cut.snapped[0].ends_with("onto the cut of `m1` at 00:00:11:12"));
}

#[test]
fn a_snapped_blade_cuts_the_clips_it_was_asked_to_cut() {
    let project = base();
    // A picture that starts on V2 between where the blade is asked for and
    // the pause it would go into: the blade would cut that too.
    let over = "track add V2 video\n\
                overwrite media=m1 src=00:05:00:00 dur=00:00:01:00 at=01:00:02:02 on=V2 as=t1\n";
    let into = known(&project, &format!("{over}blade at=01:00:01:21 snap=pause")).unwrap_err();
    assert_eq!(
        into.message,
        "`a1` would take the blade to 01:00:02:04, into the 0.60s pause of `m1` at 00:00:12:00, \
         between \"closed,\" and \"the\", where it would cut `t1` as well: blade there with at=, \
         or leave a track out with tracks="
    );
    let left = known(
        &project,
        &format!("{over}blade at=01:00:01:21 tracks=V1,A1 snap=pause"),
    )
    .unwrap();
    assert_eq!(bladed(&left), 52);
    assert_eq!(frames(left.project.clip("t1").unwrap().duration), 24);
    // And one that ends there: the blade would no longer cut it.
    let under = "track add V2 video\n\
                 overwrite media=m1 src=00:05:00:00 dur=00:00:00:14 at=01:00:01:12 on=V2 as=t1\n";
    let out = known(&project, &format!("{under}blade at=01:00:01:21 snap=cut")).unwrap_err();
    assert!(out.message.contains("no cut of `m1` within"), "{out:?}");
    let out = known(&project, &format!("{under}blade at=01:00:01:21 snap=pause")).unwrap_err();
    assert!(
        out.message.contains("where it would not cut `t1`"),
        "{out:?}"
    );
    // A track that is not there is the kernel's to name, as without snap=.
    let missing = known(&project, "blade at=01:00:01:21 tracks=A9 snap=pause").unwrap_err();
    assert_eq!(
        missing.message,
        fails(&project, "blade at=01:00:01:21 tracks=A9").message
    );
    // A clip on a sound track that plays no media has no index to go by.
    let nested = "sequence add s2 rate=24 size=1920x1080\n\
                  use s2\n\
                  track add A1 audio\n\
                  insert media=m1 src=00:02:00:00 dur=00:00:04:00 at=0f on=A1\n\
                  use s1\n\
                  track add A2 audio\n\
                  overwrite seq=s2 dur=00:00:04:00 at=01:00:00:00 on=A2 as=n1\n";
    let unheard = known(&project, &format!("{nested}blade at=01:00:01:21 snap=pause")).unwrap_err();
    assert_eq!(
        unheard.message,
        "clip `n1` sounds and plays no media to snap to: leave its track out with tracks="
    );
}

/// Facts that know only where m1's picture cuts, as an index records them.
struct Cuts(Vec<f64>);

impl Facts for Cuts {
    fn speech(&self, _: &Media) -> Speech {
        Speech::default()
    }

    fn breaks(&self, _: &Media) -> Option<Breaks> {
        Some(Breaks {
            pauses: Vec::new(),
            gaps: Vec::new(),
            cuts: self.0.clone(),
        })
    }
}

#[test]
fn snapping_lands_on_the_cut_and_inside_the_clip() {
    let project = base();
    let cut = |cuts: &[f64], script: &str| {
        apply_script_with(&project, Catalog::builtin(), &Cuts(cuts.to_vec()), script)
            .map_err(|mut e| e.remove(0))
    };
    // An index keeps microseconds: frame 290 of m1 at 24 fps, 12.0833…s, is
    // 12.083333, a hair before the frame. The edge still lands on it.
    let split = cut(&[12.083333], "split c1 at=01:00:02:00 snap=cut as=c1b").unwrap();
    assert_eq!(frames(split.project.clip("c1b").unwrap().start), 50);
    let placed = cut(
        &[11.5, 12.041667],
        "insert media=m1 src=00:00:11:12 out=00:00:12:00 snap=cut at=end on=V1 as=c3",
    )
    .unwrap();
    assert_eq!(source(&placed, "c3"), (276, 289));
    // A nearer cut before c1's in point does not count for a split inside it.
    let inside = cut(&[9.9, 10.3], "split c1 at=01:00:00:02 snap=cut as=c1b").unwrap();
    assert_eq!(frames(inside.project.clip("c1b").unwrap().start), 7);
    assert!(cut(&[9.9], "split c1 at=01:00:00:02 snap=cut")
        .unwrap_err()
        .message
        .contains("would do here"));
}

#[test]
fn snapping_moves_only_the_edges_a_line_gives() {
    let project = base();
    // Only out= is given, so only the out point moves; it is already in
    // the pause.
    let placed = known(
        &project,
        "insert media=m1 out=00:00:12:05 snap=pause at=end on=V1 as=c3",
    )
    .unwrap();
    assert_eq!(source(&placed, "c3"), (0, 293));
    assert!(
        known(&project, "insert media=m1 dur=2s snap=pause at=end on=V1")
            .unwrap_err()
            .message
            .contains("give one")
    );
    // Keyed and positional arguments mix as in any line.
    let trimmed = known(&project, r#"trim c1 src="closed" out"#).unwrap();
    assert_eq!(source(&trimmed, "c1"), (240, 288));
    // A line without media reads its fields as it always has.
    assert!(known(&project, r#"trim c9 in src="so we""#)
        .unwrap_err()
        .message
        .contains("no clip `c9`"));
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

#[test]
fn looks_are_written_like_effects_and_flatten_into_what_they_stand_for() {
    let applied = run(&base(), "fx c1 add look.shake@1 intensity=0.7 hit=12f\n");
    let clip = applied.project.clip("c1").unwrap();
    // The document keeps the one line, the time as a time.
    let printed = print_document(
        &applied.project,
        Catalog::builtin(),
        PrintOptions::default(),
    );
    assert!(
        printed.contains("      fx look.shake@1 intensity=0.7 hit=12f\n"),
        "{printed}"
    );
    assert_eq!(clip.effects.len(), 1);
    // Flattened, it is the properties and effects it expands to.
    let flat = run(&applied.project, "fx c1 flatten 1\n");
    let clip = flat.project.clip("c1").unwrap();
    assert_eq!(clip.effects.len(), 1);
    assert_eq!(clip.effects[0].fx.as_str(), "blur.directional");
    let kerf_model::Param::Animated(keys) = &clip.props["x"] else {
        panic!("x animates");
    };
    assert_eq!(frames(keys[0].at), 12);
    // A look for titles is not one for this clip.
    let error = fails(&base(), "fx c1 add look.pop-in@1\n");
    assert!(error.message.contains("a look for titles"), "{error:?}");
}

#[test]
fn a_look_s_time_can_name_a_marker_it_lands_on() {
    let applied = run(
        &base(),
        "marker add as=hit1 at=01:00:01:00\nfx c1 add look.shake@1 hit=hit1\n",
    );
    let printed = print_document(
        &applied.project,
        Catalog::builtin(),
        PrintOptions::default(),
    );
    assert!(printed.contains("fx look.shake@1 hit=hit1\n"), "{printed}");
    let flat = run(&applied.project, "fx c1 flatten 1\n");
    let kerf_model::Param::Animated(keys) = &flat.project.clip("c1").unwrap().props["x"] else {
        panic!("x animates");
    };
    assert_eq!(frames(keys[0].at), 24);
    // The marker must be the clip's sequence's.
    let error = fails(&base(), "fx c1 add look.shake@1 hit=nowhere\n");
    assert_eq!(error.code, "marker.read", "{error:?}");
}

#[test]
fn a_slower_clip_that_has_no_room_says_what_it_runs_into() {
    let error = fails(&base(), "speed c1 0.5\n");
    assert_eq!(error.code, "speed.collision", "{error:?}");
    assert!(
        error.message.contains(
            "`c1` at speed 0.5 would be 192 frames long, 96 more than it is, and run into `c2` on \
             track V1; make room after it first"
        ),
        "{error:?}"
    );
    let error = fails(&base(), "speed c1 {0f:1 24f:1/4}\n");
    assert!(
        error.message.contains("`c1` with that ramp would be"),
        "{error:?}"
    );
}

#[test]
fn a_look_stays_on_its_moment_when_its_clip_is_split_or_trimmed() {
    let printed = |applied: &Applied| {
        print_document(
            &applied.project,
            Catalog::builtin(),
            PrintOptions::default(),
        )
    };
    let pulsed = run(
        &base(),
        "fx c1 add look.zoom-pulse@1 amount=0.1 hit=60f length=10f\n",
    );
    // Split before the pulse: the second part pulses where the frame it pulsed on now is, and its
    // length is a length, which stays; the first part keeps its numbers, which trimming it back
    // brings back.
    let split = run(&pulsed.project, "split c1 at=01:00:00:20 as=c9\n");
    let text = printed(&split);
    assert!(
        text.contains("    clip c1 media=m1 src=00:00:10:00 at=01:00:00:00 dur=00:00:00:20 link=L1\n      fx look.zoom-pulse@1 amount=0.1 hit=60f length=10f\n"),
        "{text}"
    );
    assert!(
        text.contains("    clip c9 media=m1 src=00:00:10:20 at=01:00:00:20 dur=00:00:03:04 link=L3\n      fx look.zoom-pulse@1 amount=0.1 hit=40f length=10f\n"),
        "{text}"
    );
    // So does a trimmed head, and a moment left to its default is written, so that the pulse at
    // the clip's first frame stays where that frame was.
    let shaken = run(&base(), "fx c1 add look.shake@1\ntrim c1 in +6f\n");
    assert!(
        printed(&shaken).contains("fx look.shake@1 hit=-6f\n"),
        "{}",
        printed(&shaken)
    );
    // A moment that names a marker is a place in the sequence.
    let marked = run(
        &base(),
        "marker add as=beat at=01:00:02:00\nfx c1 add look.shake@1 hit=beat\nsplit c1 at=01:00:01:00\n",
    );
    assert_eq!(
        printed(&marked)
            .matches("fx look.shake@1 hit=beat\n")
            .count(),
        2
    );
    // A slow push has no moment: its push is from the clip's first frame, whichever that is.
    let pushed = run(
        &base(),
        "fx c1 add look.slow-push@1 length=2s\nsplit c1 at=01:00:01:00\n",
    );
    assert_eq!(
        printed(&pushed)
            .matches("fx look.slow-push@1 length=48f\n")
            .count(),
        2
    );
    // Transitions' looks have no moments either.
    let whip = Catalog::builtin().look("look.whip", 1).unwrap();
    assert!(whip.moments().is_empty());
    assert_eq!(
        Catalog::builtin().look("look.flash", 1).unwrap().moments(),
        ["hit"]
    );
    assert_eq!(
        Catalog::builtin().look("look.pop-in", 1).unwrap().moments(),
        ["start"]
    );
}

#[test]
fn media_replace_prints_every_field_and_replays() {
    let project = base();
    let script = r#"media replace m1 path="media/take 2-0123abcd.mov" rate=23.976 tc=14:22:31:05 dur=00:12:00:00 video=3840x2160 alpha=true audio=6ch@96000 reel="B002""#;
    let applied = run(&project, script);
    let media = applied.project.media("m1").unwrap();
    assert_eq!(media.path, "media/take 2-0123abcd.mov");
    assert_eq!(media.rate, Rate::FPS_23_976);
    assert_eq!(
        (media.start, media.drop_frame),
        parse_timecode("14:22:31:05", Rate::FPS_23_976).unwrap()
    );
    assert_eq!(media.duration, Rate::FPS_23_976.time_of(12 * 60 * 24));
    assert_eq!(
        media.video.map(|v| (v.width, v.height, v.alpha)),
        Some((3840, 2160, true))
    );
    assert_eq!(
        media.audio.map(|a| (a.channels, a.sample_rate)),
        Some((6, 96_000))
    );
    assert_eq!(media.reel.as_deref(), Some("B002"));
    // The clips keep their timing.
    assert_eq!(applied.project.sequences, project.sequences);

    let printed = print_ops(&project, Catalog::builtin(), &applied.ops);
    assert_eq!(printed, format!("{script}\n"));
    assert_eq!(run(&project, &printed).project, applied.project);

    // Drop-frame timecode, a length off the frame grid, no reel, and no
    // sound (nothing here plays m1's sound but A1, so drop that first).
    let silent = run(&project, "delete a1 a2").project;
    let op = Op::ReplaceMedia(Media {
        id: Id::new("m1").unwrap(),
        path: "b.mov".into(),
        rate: Rate::FPS_29_97,
        drop_frame: true,
        start: parse_timecode("01:00:00;00", Rate::FPS_29_97).unwrap().0,
        duration: Time::from_flicks(Rate::FPS_29_97.time_of(1800 * 2).flicks() + 1),
        video: Some(kerf_model::VideoInfo {
            width: 1280,
            height: 720,
            alpha: false,
            ..Default::default()
        }),
        audio: None,
        reel: None,
        recipe: None,
        steps: Vec::new(),
        sync: None,
    });
    let printed = print_op(&silent, Catalog::builtin(), &op);
    assert!(printed.contains("tc=01:00:00;00"), "{printed}");
    assert!(printed.ends_with("fl video=1280x720"), "{printed}");
    assert_eq!(run(&silent, &printed).ops, vec![op]);
}

#[test]
fn a_media_is_lined_up_with_another_recording_of_the_same_thing() {
    let add = r#"media add m2 path="recorder.wav" rate=24 dur=00:10:00:00 audio=2ch@48000
media add m3 path="b-cam.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000 sync=m1@-2.5s"#;
    let project = run(&base(), add).project;
    // A camera's sound is finer than its frames: the offset is in seconds, to the millisecond.
    let script = "media sync m2 with=m1@1.234s";
    let applied = run(&project, script);
    let printed = print_document(&applied.project, Catalog::builtin(), PrintOptions::default());
    assert!(
        printed.contains(
            "media m2 path=\"recorder.wav\" rate=24 dur=00:10:00:00 audio=2ch@48000 sync=m1@1.234s\n"
        ),
        "{printed}"
    );
    assert!(printed.contains(" sync=m1@-2.5s\n"), "{printed}");
    assert_eq!(
        parse_document(&printed, Catalog::builtin()).unwrap().project,
        applied.project
    );
    assert_eq!(print_ops(&project, Catalog::builtin(), &applied.ops), format!("{script}\n"));
    // A moment of one is the moment of the other that far on, through each it names in turn.
    let chained = run(&applied.project, "media sync m3 with=m2@0.5s").project;
    let seconds = |id: &str| {
        let (with, at) = chained.synced_to(id).unwrap();
        (with.to_string(), at.seconds())
    };
    assert_eq!(seconds("m1"), ("m1".to_owned(), 0.0));
    assert_eq!(seconds("m2"), ("m1".to_owned(), 1.234));
    assert_eq!(seconds("m3"), ("m1".to_owned(), 1.734));
    // `with=""` lets go, as `parent=""` does, and prints as it was written.
    let free = run(&applied.project, "media sync m2 with=\"\"");
    assert_eq!(free.project.media("m2").unwrap().sync, None);
    assert_eq!(
        print_ops(&applied.project, Catalog::builtin(), &free.ops),
        "media sync m2 with=\"\"\n"
    );
    // An offset that is no whole number of milliseconds is kept exactly, in flicks.
    let exact = run(&project, "media sync m2 with=m1@7fl").project;
    let printed = print_document(&exact, Catalog::builtin(), PrintOptions::default());
    assert!(printed.contains(" sync=m1@7fl\n"), "{printed}");

    for (script, code, says) in [
        ("media sync m2 with=m9@1s", "ref.missing", "no media `m9`"),
        ("media sync m9 with=m1@1s", "ref.missing", "no media `m9`"),
        ("media sync m2 with=m2@1s", "sync.ring", "cannot be lined up with `m2`"),
        // m3 is lined up with m1 already.
        ("media sync m1 with=m3@1s", "sync.ring", "which is lined up with it"),
        ("media sync m2 with=m1", "syntax", "as in `m1@1.5s` or `m1@-0.25s`"),
        ("media sync m2 with=m1@30f", "syntax", "in seconds"),
        ("media sync m2", "syntax", "missing `with=`"),
    ] {
        let error = fails(&project, script);
        assert_eq!(error.code, code, "{script}: {error:?}");
        assert!(error.message.contains(says), "{script}: {error:?}");
    }
    // A document that goes round, or names a media it has not, does not open.
    let ring = printed.replace("audio=2ch@48000\nmedia m2", "audio=2ch@48000 sync=m2@0s\nmedia m2");
    assert_ne!(ring, printed);
    let errors = parse_document(&ring, Catalog::builtin()).unwrap_err();
    assert!(errors.iter().any(|error| error.code == "sync.ring"), "{errors:#?}");
    let missing = printed.replace("sync=m1@7fl", "sync=m9@7fl");
    let errors = parse_document(&missing, Catalog::builtin()).unwrap_err();
    assert!(
        errors[0].message.contains("lined up with unknown media `m9`"),
        "{errors:#?}"
    );
    // Removing what a media is lined up with lets it go, and says so.
    let alone = run(&chained, "delete c1 c2\nmedia remove m1");
    assert_eq!(alone.project.media("m2").unwrap().sync, None);
    assert!(alone.project.media("m3").unwrap().sync.is_some());
    assert!(
        alone.report.notes.iter().any(|note| note.contains("`m2` was lined up with `m1`")),
        "{:?}",
        alone.report.notes
    );
}

#[test]
fn media_replace_names_the_clips_a_shorter_file_cannot_hold() {
    // c2 and a2 play 00:01:00:00–00:01:03:00 of m1.
    let error = fails(
        &base(),
        r#"media replace m1 path="short.mov" rate=24 dur=00:00:30:00 video=1920x1080 audio=2ch@48000"#,
    );
    assert_eq!(
        (error.code, error.span.map(|span| span.line)),
        ("clip.source_range", Some(1))
    );
    assert!(
        error.message.contains("`a2`") || error.message.contains("`c2`"),
        "{error:?}"
    );
    let error = fails(&base(), r#"media replace m1 path="a.mov" rate=24"#);
    assert!(error.message.contains("missing `dur=`"), "{error:?}");
    let error = fails(&base(), "media swap m1");
    assert!(error.message.contains("replace"), "{error:?}");
}

/// A media made by a model, with the scene its guide is drawn from: the steps are written
/// under the media by `step` lines, changed a name at a time, and read back as the
/// document has them.
#[test]
fn steps_are_written_under_the_media_they_make() {
    let applied = run(
        &base(),
        r#"
media add depth path="shots/depth.mp4" rate=16 dur=65f video=832x480
step depth add render s1 {guide.mp4} size=832x480 rate=16 pass=depth
step depth add generate seed=7 model=shot guide={guide.mp4} prompt="a red car on a wet street" seconds=4
"#,
    );
    let document = print_document(&applied.project, Catalog::builtin(), PrintOptions::default());
    let media = "media depth path=\"shots/depth.mp4\" rate=16 dur=00:00:04:01 video=832x480\n\
        \x20 render s1 {guide.mp4} pass=depth size=832x480 rate=16\n\
        \x20 generate model=shot prompt=\"a red car on a wet street\" guide={guide.mp4} seconds=4 seed=7\n";
    assert!(document.contains(media), "{document}");
    // The document reads as it prints, steps and all.
    let read = parse_document(&document, Catalog::builtin()).unwrap().project;
    assert_eq!(read, applied.project);
    // And its history replays: each operation prints as the line that makes it.
    let history = print_ops(&base(), Catalog::builtin(), &applied.ops);
    assert!(
        history.contains("step depth add render s1 {guide.mp4} pass=depth size=832x480 rate=16\n"),
        "{history}"
    );
    assert_eq!(run(&base(), &history).project, applied.project);

    // One name set leaves the rest of the step as it was; a step is put before another
    // by its number; one whose file a later step reads stays until that step lets go.
    let project = applied.project;
    let changed = run(
        &project,
        r#"
step depth set 2 seed=8 strength=0.6
step depth add 1 generate model=still prompt="a red sports car" out={car.png}
step depth set 3 ref={car.png}
"#,
    )
    .project;
    let steps: Vec<String> = changed.media("depth").unwrap().steps.iter().map(crate::print_step).collect();
    assert_eq!(
        steps,
        [
            "generate model=still prompt=\"a red sports car\" out={car.png}",
            "render s1 {guide.mp4} pass=depth size=832x480 rate=16",
            "generate model=shot prompt=\"a red car on a wet street\" guide={guide.mp4} ref={car.png} seconds=4 seed=8 strength=0.6",
        ]
    );
    let error = fails(&changed, "step depth remove 1");
    assert_eq!(error.code, "step.order");
    assert!(error.message.contains("step 2 reads `{car.png}`"), "{error}");
    let without = run(&changed, "step depth set 3 ref=\"\"\nstep depth remove 1").project;
    assert_eq!(without.media("depth").unwrap().steps.len(), 2);
    // A step written whole replaces the one there.
    let whole = run(&changed, "step depth set 2 render s1 {guide.mp4} pass=ids").project;
    assert_eq!(
        crate::print_step(&whole.media("depth").unwrap().steps[1]),
        "render s1 {guide.mp4} pass=ids"
    );

    // Choosing another file for the media leaves how it is made as it is.
    let take = run(
        &changed,
        "media replace depth path=\"shots/depth.takes/001.mp4\" rate=16 dur=65f video=832x480",
    )
    .project;
    assert_eq!(take.media("depth").unwrap().path, "shots/depth.takes/001.mp4");
    assert_eq!(take.media("depth").unwrap().steps, changed.media("depth").unwrap().steps);
}

#[test]
fn a_step_that_cannot_be_is_refused_where_it_is_written() {
    let project = run(
        &base(),
        "media add gone path=\"shots/gone.mp4\" rate=16 dur=81f video=832x480\n\
         step gone add generate model=matte guide=footage/walk.mp4 prompt=\"man\" out={man.mp4}\n\
         step gone add generate model=plate guide=footage/walk.mp4 guide_mask={man.mp4} reach=72",
    )
    .project;
    for (script, says) in [
        ("step gone add run vork render", "`run` is not a step"),
        ("step gone add generate model=plate guide={man.mp4}", "writes `{out}`, the media's file, so it is the last"),
        ("step gone add 1 generate model=plate guide_mask={man.mp4} out={a.mp4}", "no step before this one makes `{man.mp4}`"),
        ("step gone add 5 generate model=plate out={a.mp4}", "a step added is step 3 at most"),
        ("step gone set 3 seed=8", "has 2 steps and no step 3"),
        ("step gone set 2 seed=eight", "not a whole number"),
        ("step gone set 2", "say what step 2 is to be"),
        ("step gone remove 2 now", "unexpected argument"),
        ("step gone move 1", "expected add, set, or remove"),
        ("step m9 add render s1 {out}", "no media `m9`"),
        ("step gone add generate model=plate prompt=\"a\\tb\"", "a step is one line"),
    ] {
        let error = fails(&project, script);
        assert!(error.message.contains(says), "{script}: {error}");
    }
    // A media that names a recipe file has its steps there, and one with steps names none.
    let filed = run(
        &base(),
        "media add m3 path=\"shots/open.mp4\" rate=24 dur=96f video=320x180 recipe=\"shots/open.kerfrecipe\"",
    )
    .project;
    let error = fails(&filed, "step m3 add render s1 {out}");
    assert_eq!(error.code, "step.recipe");
    let error = fails(
        &project,
        "media replace gone path=\"shots/gone.mp4\" rate=16 dur=81f video=832x480 recipe=\"shots/gone.kerfrecipe\"",
    );
    assert_eq!(error.code, "step.recipe");
}

#[test]
fn a_document_says_what_is_wrong_under_a_media() {
    let document = |under: &str| {
        format!(
            "kerf 1\nproject name=\"t\"\nmedia m3 path=\"shots/open.mp4\" rate=24 dur=96f video=320x180\n{under}sequence s1 rate=24 size=320x180\n"
        )
    };
    let wrong = |under: &str| {
        parse_document(&document(under), Catalog::builtin())
            .expect_err("expected the document to be refused")
            .remove(0)
    };
    assert!(wrong("  track V1 video\n").message.contains("under a media come the steps that make its file"));
    assert!(wrong("  generate prompt=\"a boat\"\n").message.contains("names its model"));
    let error = wrong("  generate model=dawn guide={guide.mp4}\n");
    assert!(error.message.contains("no step before this one makes `{guide.mp4}`"), "{error}");
    assert_eq!(error.span.map(|span| span.line), Some(4));
    let error = wrong("  render s1 {out}\n  generate model=dawn\n");
    assert_eq!(error.code, "step.order");
    // Steps that stop short of the media's file read: the making says what is missing.
    let read = parse_document(
        &document("  render s1 {guide.mp4} pass=depth\n"),
        Catalog::builtin(),
    )
    .unwrap();
    assert_eq!(read.project.media[0].steps.len(), 1);
}
