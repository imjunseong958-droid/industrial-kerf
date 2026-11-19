use super::*;
use crate::doc::parse_document;
use crate::script::{apply_script, print_ops};

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

fn script_of(current: &Project, forwarded: &Forwarded) -> String {
    print_ops(current, Catalog::builtin(), &forwarded.ops)
}

#[test]
fn a_ripple_after_another_ripple_leaves_no_gap() {
    // Merging the two edits by id keeps one side's place for c4 and leaves
    // it half a second after c3. Run again, the trim closes up to it.
    let current = edited("trim c1 out -24f ripple");
    let forwarded = forward(&current, "trim c3 out -12f ripple");
    assert_eq!(span(&forwarded.project, "c3"), (168, 252));
    assert_eq!(span(&forwarded.project, "c4"), (252, 348));
    assert_eq!(span(&forwarded.project, "a4"), (252, 348));
    assert_eq!(script_of(&current, &forwarded), "trim c3 out -12f ripple\n");
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
    assert_eq!(second.message, "clip `c4` was slipped since this was proposed");
    assert_eq!(second.span.map(|span| span.line), Some(2));
    assert_eq!(second.entity.as_ref().map(Id::as_str), Some("c4"));
}

#[test]
fn a_place_moves_with_the_clips_that_held_it() {
    // Everything from the second clip on is a second earlier.
    let current = edited("trim c1 out -24f ripple");
    // The cut between c2 and c3; then, each as the lines before leave the
    // project, a time inside c3, another, and the end.
    let forwarded = forward(
        &current,
        "insert gen=solid at=192f dur=24f on=V1 as=g1\n\
         marker add as=mk1 at=240f\n\
         blade at=250f tracks=V1\n\
         overwrite gen=solid at=408f dur=24f on=V2 as=g2",
    );
    assert_eq!(span(&forwarded.project, "g1"), (168, 192));
    assert_eq!(span(&forwarded.project, "c3"), (192, 226));
    assert_eq!(span(&forwarded.project, "g2"), (384, 408));
    let sequence = &forwarded.project.sequences[0];
    assert_eq!(
        Rate::FPS_24.frames_exact(sequence.marker("mk1").unwrap().at),
        Some(216)
    );
    // What is staged again is the script with its places moved, and it
    // makes the same project.
    let script = script_of(&current, &forwarded);
    assert_eq!(
        script,
        "insert gen=solid@1 at=00:00:07:00 dur=00:00:01:00 on=V1 as=g1\n\
         marker add as=mk1 at=00:00:09:00\n\
         blade at=00:00:09:10 tracks=V1\n\
         overwrite gen=solid@1 at=00:00:16:00 dur=00:00:01:00 on=V2 as=g2\n"
    );
    let again = apply_script(&current, Catalog::builtin(), &script).unwrap();
    assert_eq!(again.project, forwarded.project);
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
fn a_time_no_clip_held_stays_where_it_is() {
    let current = edited("trim c1 out -24f ripple");
    let forwarded = forward(&current, "overwrite gen=solid at=480f dur=24f on=V2 as=g1");
    assert_eq!(span(&forwarded.project, "g1"), (480, 504));
}

#[test]
fn clips_the_script_makes_are_followed_by_the_names_they_get_now() {
    // On the base the split makes c5 and its sound c6. The edited project
    // has both already.
    let current = edited("split c1 at=48f");
    assert!(current.clip("c5").is_some() && current.clip("c6").is_some());
    let forwarded = forward(&current, "split c3 at=240f\ntrim c5 out -12f\nset c5 opacity=0.5");
    assert_eq!(
        script_of(&current, &forwarded),
        "split c3 at=00:00:10:00\ntrim c7 out -12f\nset c7 opacity=0.5\n"
    );
    assert_eq!(span(&forwarded.project, "c7"), (240, 276));
    assert_eq!(span(&forwarded.project, "c8"), (240, 276), "its sound is cut with it");
    assert_eq!(span(&forwarded.project, "c5"), (48, 96), "the other edit's c5 is left alone");
}

#[test]
fn markers_and_transitions_the_script_makes_are_followed_too() {
    // The edited project has a marker and a transition of its own, with the
    // ids the script's would have had.
    let current = edited("marker add at=10f\ntransition add from=c1 to=c2 fx=dissolve dur=12f");
    let made = &current.sequences[0];
    assert!(made.marker("mk1").is_some() && made.tracks[0].transition("t1").is_some());
    let forwarded = forward(
        &current,
        "marker add at=192f\nmarker set mk1 name=\"cut\"\n\
         transition add from=c3 to=c4 fx=dissolve dur=12f\ntransition set t1 dur=6f",
    );
    let sequence = &forwarded.project.sequences[0];
    assert_eq!(sequence.marker("mk1").unwrap().name, None, "the other edit's marker");
    assert_eq!(sequence.marker("mk2").unwrap().name.as_deref(), Some("cut"));
    let frames = |id: &str| {
        Rate::FPS_24.frames_exact(sequence.tracks[0].transition(id).unwrap().duration)
    };
    assert_eq!((frames("t1"), frames("t2")), (Some(12), Some(6)));
}

#[test]
fn a_change_to_how_a_clip_looks_does_not_stop_a_cut_of_it() {
    let current = edited("set c3 opacity=0.5\nfx c3 add color.basic exposure=0.3");
    let forwarded = forward(&current, "trim c3 out -12f ripple\nsplit c2 at=120f");
    assert_eq!(span(&forwarded.project, "c3"), (192, 276));
    // But a line that sets what was set since is not the edit it was.
    let set = refused(&current, "set c3 opacity=0.8");
    assert_eq!(set.code, "forward.changed");
    assert_eq!(
        set.message,
        "the properties of clip `c3` have changed since this was proposed"
    );
    let effect = refused(&current, "fx c3 add color.basic exposure=-0.2");
    assert_eq!(
        effect.message,
        "the effects of clip `c3` have changed since this was proposed"
    );
    // A property the other edit left alone is set on a clip it only moved.
    let moved = edited("trim c1 out -24f ripple");
    assert_eq!(span(&forward(&moved, "set c3 opacity=0.8").project, "c3"), (168, 264));
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

#[test]
fn lines_that_name_no_clip_and_no_place_run_as_they_are() {
    let current = edited("trim c3 out -24f ripple\nsequence set s1 size=1280x720");
    let forwarded = forward(&current, "track add V3 video\nmarker add as=mk1 at=0f name=\"top\"");
    assert!(forwarded.project.sequences[0].track("V3").is_some());
    assert_eq!(forwarded.project.sequences[0].width, 1280);
    assert_eq!(
        script_of(&current, &forwarded),
        "track add V3 video\nmarker add as=mk1 at=00:00:00:00 name=\"top\"\n"
    );
}

#[test]
fn what_the_edited_project_refuses_is_refused_with_its_reason() {
    // The other edit took the id the script gives its clip.
    let taken = edited("insert gen=solid at=384f dur=24f on=V1 as=g1");
    let error = refused(&taken, "overwrite gen=solid at=0f dur=24f on=V2 as=g1");
    assert_eq!(error.code, "id.taken");
    assert_eq!(error.span.map(|span| span.line), Some(1));
}
