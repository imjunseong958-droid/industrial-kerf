use super::*;
use crate::doc::parse_document;
use crate::script::apply_script;

/// Four clips of four seconds, end to end, each linked to its sound.
const BASE: &str = r#"kerf 1
project name="t"

media m1 path="a.mov" rate=24 dur=00:10:00:00 video=1920x1080 audio=2ch@48000

sequence s1 name="Main" rate=24 size=1920x1080 audio=48000
  track V1 video
    clip c1 media=m1 src=00:00:10:00 at=0f dur=96f link=L1
    clip c2 media=m1 src=00:01:00:00 at=96f dur=96f link=L2
    clip c3 media=m1 src=00:02:00:00 at=192f dur=96f link=L3
    clip c4 media=m1 src=00:03:00:00 at=288f dur=96f link=L4
  track V2 video
  track A1 audio
    clip a1 media=m1 src=00:00:10:00 at=0f dur=96f link=L1
    clip a2 media=m1 src=00:01:00:00 at=96f dur=96f link=L2
    clip a3 media=m1 src=00:02:00:00 at=192f dur=96f link=L3
    clip a4 media=m1 src=00:03:00:00 at=288f dur=96f link=L4
"#;

fn base() -> Project {
    parse_document(BASE, Catalog::builtin()).unwrap().project
}

/// The base as `script` edits it: what was accepted while a proposal waited.
fn edited(script: &str) -> Project {
    apply_script(&base(), Catalog::builtin(), script)
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
        .project
}

fn forward(current: &Project, script: &str) -> Forwarded {
    bring_forward(&base(), current, Catalog::builtin(), script)
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
}

fn refused(current: &Project, script: &str) -> Diagnostic {
    bring_forward(&base(), current, Catalog::builtin(), script)
        .expect_err("expected the script to be refused")
        .remove(0)
}

/// Where a clip starts and ends, in frames.
fn span(project: &Project, id: &str) -> (i64, i64) {
    let clip = project.clip(id).unwrap_or_else(|| panic!("no clip {id}"));
    let frames = |time: Time| Rate::FPS_24.frames_exact(time).unwrap();
    (frames(clip.start), frames(clip.end()))
}

#[test]
fn a_line_is_refused_for_a_clip_that_was_cut_moved_or_removed() {
    let script = "trim c3 out -12f ripple";
    let cut = refused(&edited("trim c3 in +6f"), script);
    assert_eq!(cut.code, "forward.changed");
    assert_eq!(cut.message, "clip `c3` was cut since this was proposed");
    assert_eq!(cut.entity.as_ref().map(Id::as_str), Some("c3"));
    assert_eq!(cut.span.map(|span| span.line), Some(1));

    let moved = refused(&edited("unlink c3 a3\nmove c3 on=V2"), script);
    assert_eq!(
        moved.message,
        "clip `c3` was moved to track V2 since this was proposed"
    );

    let gone = refused(&edited("delete c3"), script);
    assert_eq!(gone.code, "forward.gone");
    assert_eq!(gone.message, "clip `c3` is gone since this was proposed");

    // The second line is the one that is stopped.
    let second = refused(
        &edited("slip c4 +6f"),
        "trim c2 out -12f ripple\ntrim c4 out -12f",
    );
    assert_eq!(
        second.message,
        "clip `c4` was slipped since this was proposed"
    );
    assert_eq!(second.span.map(|span| span.line), Some(2));
    assert_eq!(second.entity.as_ref().map(Id::as_str), Some("c4"));
}

#[test]
fn a_place_is_refused_when_what_held_it_changed() {
    // 200f is inside c3 and a3.
    let cut = refused(&edited("trim c3 out -6f"), "marker add at=200f");
    assert_eq!(cut.code, "forward.changed");
    assert_eq!(
        cut.message,
        "00:00:08:08 was on clip `c3`, which was cut since this was proposed"
    );
    let gone = refused(&edited("unlink c3 a3\ndelete a3"), "marker add at=200f");
    assert_eq!(gone.code, "forward.gone");
    assert_eq!(
        gone.message,
        "00:00:08:08 was on clip `a3`, which is gone since this was proposed"
    );
    // Picture closed up and sound did not: the cut between the second and
    // third clips is at two times now.
    let apart = refused(
        &edited("unlink c1 a1\ntrack set A1 sync=off\ntrim c1 out -24f ripple"),
        "marker add at=192f",
    );
    assert_eq!(apart.code, "forward.place");
    assert_eq!(
        apart.message,
        "00:00:08:00 was on clips `c2` and `a2`, which are no longer together there"
    );
}

#[test]
fn a_line_is_refused_when_it_would_change_other_clips_than_it_did() {
    // The roll moved the cut between c2 and c3. With a clip put between
    // them, it would trim that one.
    let between = edited("insert gen=solid at=192f dur=24f on=V1 as=g1");
    let roll = refused(&between, "roll c2 +6f");
    assert_eq!(roll.code, "forward.differs");
    assert_eq!(
        roll.message,
        "this line would now change clip `g1`, which was not there when it was proposed"
    );
    // The lift cleared from c2 to c3, and would now take a title with it.
    let title = edited("overwrite gen=solid at=120f dur=24f on=V2 as=g1");
    let lift = refused(&title, "lift from=100f to=200f");
    assert_eq!(
        lift.message,
        "this line would now change clip `g1`, which was not there when it was proposed"
    );
    // Named for V1 alone, it leaves the title be.
    let forwarded = forward(&title, "lift from=100f to=200f tracks=V1");
    assert_eq!(span(&forwarded.project, "c2"), (96, 100));
    assert_eq!(span(&forwarded.project, "g1"), (120, 144));
    // An insert at a cut opened time and cut nothing. A title put over that
    // cut since would be cut in two by it.
    let over = edited("overwrite gen=solid at=150f dur=60f on=V2 as=g1");
    let insert = refused(&over, "insert gen=solid at=192f dur=24f on=V1 as=g2");
    assert_eq!(
        insert.message,
        "this line would now change clip `g1`, which was not there when it was proposed"
    );
    // A clip the line cut that is not there to be cut.
    let gone = refused(&edited("unlink c2 a2\ndelete a2"), "blade at=120f");
    assert_eq!(
        gone.message,
        "00:00:05:00 was on clip `a2`, which is gone since this was proposed"
    );
}
