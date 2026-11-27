//! A clip that follows another: how a document writes it, and what each edit does to it.

use kerf_lang::{apply_script, parse_document, print_document, print_ops, PrintOptions};
use kerf_model::{Catalog, Clip, Content, Project, Rate, Time};
use proptest::prelude::*;
use proptest::sample::Index;

/// Three shots in a row, a title over the second and one over the third that follow them,
/// and music under all three that follows the first.
const EDIT: &str = "kerf 1
project name=\"t\"

media m path=\"a.mov\" rate=24 dur=00:03:20:00 video=64x36 audio=2ch@48000
media mu path=\"music.wav\" rate=24 dur=00:03:20:00 audio=2ch@48000

sequence s1 name=\"\" rate=24 size=64x36 audio=48000 start=00:00:00:00
  track V1 video
    clip c1 media=m src=00:00:10:00 at=00:00:00:00 dur=00:00:02:00
    clip c2 media=m src=00:00:20:00 at=00:00:02:00 dur=00:00:02:00
    clip c3 media=m src=00:00:30:00 at=00:00:04:00 dur=00:00:02:00
  track V2 video
    clip t2 gen=title@1 at=c2.start+12f dur=00:00:01:00
      set text=\"Two\"
    clip t3 gen=title@1 at=c3.start+6f dur=00:00:00:12
      set text=\"Three\"
  track A1 audio
    clip mus media=mu src=00:00:00:00 at=c1.start dur=00:00:06:00
";

/// A follower of the second shot that starts over its first frames, on a track of its own.
const OVER_THE_HEAD: &str =
    "track add V3 video after=V2\noverwrite gen=solid dur=3f at=c2.start+2f on=V3 as=h\nattach h to=c2";

fn project() -> Project {
    read(EDIT)
}

fn read(document: &str) -> Project {
    parse_document(document, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

fn printed(project: &Project) -> String {
    print_document(project, Catalog::builtin(), PrintOptions::default())
}

fn run(project: &Project, script: &str) -> Project {
    apply_script(project, Catalog::builtin(), script)
        .unwrap_or_else(|errors| panic!("`{script}`: {errors:?}"))
        .project
}

fn refused(project: &Project, script: &str) -> String {
    match apply_script(project, Catalog::builtin(), script) {
        Ok(_) => panic!("`{script}` ran"),
        Err(errors) => errors
            .iter()
            .map(|error| format!("{} {}", error.code, error.message))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// The frame a clip starts on.
fn start(project: &Project, id: &str) -> i64 {
    let clip = project.clip(id).unwrap_or_else(|| panic!("no `{id}`"));
    Rate::FPS_24.frames_exact(clip.start).expect("on a frame")
}

/// How a document writes where a clip is.
fn at(project: &Project, id: &str) -> String {
    let document = printed(project);
    let line = document
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("clip {id} ")))
        .unwrap_or_else(|| panic!("no line for `{id}` in\n{document}"));
    line.split_whitespace()
        .find_map(|word| word.strip_prefix("at="))
        .expect("an at=")
        .to_owned()
}

#[test]
fn a_document_writes_a_follower_by_the_clip_it_follows_and_reads_it_back() {
    let project = project();
    assert_eq!(
        [
            start(&project, "t2"),
            start(&project, "t3"),
            start(&project, "mus")
        ],
        [60, 102, 0]
    );
    assert_eq!(printed(&project), EDIT);
    // Whichever of the two comes first in the document, before its start as well as after,
    // and a follower of a follower.
    let project = read(
        "kerf 1\nproject name=\"t\"\nmedia m path=\"a.mov\" rate=24 dur=4800f video=64x36\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip early gen=solid@1 at=late.start-24f dur=12f\n    clip c1 media=m src=0f at=48f dur=48f\n  track V2 video\n    clip late gen=solid@1 at=c1.start+12f dur=12f\n    clip last gen=solid@1 at=early.start+60f dur=12f\n",
    );
    assert_eq!(
        [
            start(&project, "late"),
            start(&project, "early"),
            start(&project, "last")
        ],
        [60, 36, 96]
    );
    assert_eq!(read(&printed(&project)), project);
    // Followers written ahead of the clips they come after on their track go in among them.
    let project = read(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip f1 gen=solid@1 at=r1.start+5f dur=2f\n    clip f2 gen=solid@1 at=r1.start+90f dur=2f\n    clip r1 gen=solid@1 at=10f dur=2f\n    clip r2 gen=solid@1 at=20f dur=2f\n",
    );
    let order: Vec<&str> = project.sequences[0].tracks[0]
        .clips
        .iter()
        .map(|clip| clip.id.as_str())
        .collect();
    assert_eq!(order, ["r1", "f1", "r2", "f2"]);
}

#[test]
fn a_document_refuses_a_follower_of_no_clip_of_itself_or_of_an_end() {
    let with = |at: &str| {
        let document = format!(
            "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip a gen=solid@1 at={at} dur=12f\n    clip b gen=solid@1 at=a.start+24f dur=12f\n"
        );
        let errors = parse_document(&document, Catalog::builtin()).expect_err("refused");
        errors
            .iter()
            .map(|error| {
                let at = error.span.expect("a place in the document");
                format!("{}:{} {}", at.line, at.column, error.message)
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    assert_eq!(
        with("gone.start+2f"),
        "5:24 `a` follows `gone`, not a clip of its sequence"
    );
    let each_other = with("b.start+2f");
    assert!(
        each_other.contains("5:24 `a` follows itself through `b`")
            && each_other.contains("6:24 `b` follows itself through `a`"),
        "{each_other}"
    );
    assert!(with("b.end").contains("a clip follows another from its start"));
    assert!(with("b.start+soon").contains("after `<clip>.start` comes an amount"));
}

#[test]
fn what_moves_a_clip_takes_its_followers_and_leaves_their_lines_as_they_were() {
    let project = project();
    // A move, a slide, and the time a ripple opens or closes before it.
    for (script, moved, by) in [
        ("move c3 by=+24f", "t3", 24),
        ("slide c2 +6f", "t2", 6),
        ("ripple-delete c1", "t2", -48),
        (
            "insert media=m src=100f dur=24f at=c2.start on=V1",
            "t2",
            24,
        ),
        ("extract from=0f to=12f tracks=V1", "t3", -12),
        ("trim c1 out -12f ripple", "t3", -12),
    ] {
        let after = run(&project, script);
        assert_eq!(
            start(&after, moved),
            start(&project, moved) + by,
            "`{script}`"
        );
        assert_eq!(at(&after, moved), at(&project, moved), "`{script}`");
    }
    // And a follower of a follower goes with it.
    let chained = run(&project, "attach t3 to=t2");
    assert_eq!(at(&chained, "t3"), "t2.start+42f");
    let after = run(&chained, "slide c2 +6f");
    assert_eq!([start(&after, "t2"), start(&after, "t3")], [66, 108]);
}

#[test]
fn what_changes_a_clip_where_it_stands_leaves_its_followers_where_they_are() {
    let project = project();
    for (script, written) in [
        ("trim c2 in +6f", "c2.start+6f"),
        ("trim c2 out -6f", "c2.start+12f"),
        ("roll c1 +6f", "c2.start+6f"),
        ("slip c2 +6f", "c2.start+12f"),
        ("speed c2 2", "c2.start+12f"),
    ] {
        let after = run(&project, script);
        assert_eq!(start(&after, "t2"), 60, "`{script}`");
        assert_eq!(at(&after, "t2"), written, "`{script}`");
    }
    // A ripple trim takes time off the head, and what the clip shows after it comes
    // earlier: a follower over that comes with it, and one over the head that went stays.
    let over_the_head = run(&project, OVER_THE_HEAD);
    for script in [
        "trim c2 in +6f ripple",
        "extract from=c2.start to=c2.start+6f tracks=V1",
    ] {
        let after = run(&over_the_head, script);
        assert_eq!(
            [start(&after, "c2"), start(&after, "t2"), start(&after, "h")],
            [48, 54, 50],
            "`{script}`"
        );
    }
}

#[test]
fn a_ripple_edit_passes_under_a_follower() {
    let project = project();
    // The music runs under where time is opened and closed, and is neither cut nor moved.
    for script in [
        "insert media=m src=100f dur=24f at=c2.start on=V1",
        "ripple-delete c2",
        "trim c2 out -12f ripple",
    ] {
        let after = run(&project, script);
        let music = after.clip("mus").expect("the music");
        assert_eq!(music, project.clip("mus").expect("the music"), "`{script}`");
        assert_eq!(after.sequences[0].tracks[2].clips.len(), 1, "`{script}`");
    }
    // Following nothing, it is in the way of one and cut by another.
    let loose = run(&project, "detach mus");
    assert_eq!(at(&loose, "mus"), "00:00:00:00");
    assert!(refused(&loose, "ripple-delete c2").contains("ripple.blocked"));
    let cut = run(&loose, "insert media=m src=100f dur=24f at=c2.start on=V1");
    assert_eq!(cut.sequences[0].tracks[2].clips.len(), 2);
    // A follower's own trim and removal ripple nothing.
    for script in ["ripple-delete t2", "trim t2 in +6f ripple"] {
        let after = run(&project, script);
        assert_eq!(
            [start(&after, "c3"), start(&after, "t3")],
            [96, 102],
            "`{script}`"
        );
    }
}

#[test]
fn a_follower_follows_the_part_of_a_cut_clip_under_its_start() {
    let project = run(&project(), OVER_THE_HEAD);
    let after = run(&project, "split c2 at=c2.start+6f as=c2b");
    assert_eq!(
        [at(&after, "h"), at(&after, "t2")],
        ["c2.start+2f", "c2b.start+6f"]
    );
    assert_eq!([start(&after, "h"), start(&after, "t2")], [50, 60]);
    // Time opened inside the clip it follows moves the part after it, and what is over that.
    let after = run(
        &project,
        "insert media=m src=100f dur=24f at=c2.start+6f on=V1",
    );
    assert_eq!([start(&after, "h"), start(&after, "t2")], [50, 84]);
    // A follower cut with the clip it follows: each part follows the part it is over.
    let after = run(&project, "blade at=c2.start+18f");
    let pieces: Vec<(String, String)> = after.sequences[0].tracks[1]
        .clips
        .iter()
        .filter(|clip| {
            clip.start == Rate::FPS_24.time_of(60) || clip.start == Rate::FPS_24.time_of(66)
        })
        .map(|clip| (clip.id.to_string(), at(&after, clip.id.as_str())))
        .collect();
    let tail = after.sequences[0].tracks[0].clips[2].id.to_string();
    assert_eq!(
        pieces,
        [
            ("t2".to_owned(), "c2.start+12f".to_owned()),
            (pieces[1].0.clone(), format!("{tail}.start")),
        ]
    );
}

#[test]
fn with_the_clip_it_follows_removed_a_follower_stays_and_follows_the_one_in_its_place() {
    let project = project();
    for (script, follower, written, gone) in [
        // The next on that track, where it leaves a gap.
        ("delete c2", "t2", "c3.start-36f", "c2"),
        // The one that came up into its place.
        ("ripple-delete c1", "mus", "c2.start", "c1"),
        // The one laid over it.
        (
            "overwrite media=m src=100f dur=48f at=c2.start on=V1 as=n",
            "t2",
            "n.start+12f",
            "c2",
        ),
    ] {
        let applied = apply_script(&project, Catalog::builtin(), script).expect("runs");
        assert_eq!(
            start(&applied.project, follower),
            start(&project, follower),
            "`{script}`"
        );
        assert_eq!(at(&applied.project, follower), written, "`{script}`");
        let written = written.split('.').next().expect("a clip");
        assert_eq!(
            applied.report.notes,
            [format!(
                "`{follower}` follows `{written}` now: `{gone}` is gone"
            )],
            "`{script}`"
        );
    }
    // With no clip left on that track they follow nothing: the three that by then follow
    // the one shot left.
    let alone = run(&project, "delete c1 c3");
    let applied = apply_script(&alone, Catalog::builtin(), "delete c2").expect("runs");
    assert_eq!(at(&applied.project, "t2"), "00:00:02:12");
    assert_eq!(
        applied.report.notes,
        ["mus", "t2", "t3"].map(|clip| format!("`{clip}` follows nothing now: `c2` is gone"))
    );
}

#[test]
fn a_ripple_delete_takes_out_the_followers_wholly_over_what_it_removes() {
    let project = project();
    let applied = apply_script(&project, Catalog::builtin(), "ripple-delete c2").expect("runs");
    assert_eq!(applied.report.removed, ["c2", "t2"]);
    assert_eq!(
        applied.report.notes,
        ["took `t2` out with `c2`, which it followed and was wholly over"]
    );
    // The title of the shot after comes up with its shot, into where that one was.
    assert_eq!(
        [start(&applied.project, "c3"), start(&applied.project, "t3")],
        [48, 54]
    );
    // One that runs on past the end of what is removed stays, and follows what came up.
    let longer = run(&project, "delete t3\ntrim t2 out +36f");
    let after = run(&longer, "ripple-delete c2");
    assert_eq!(start(&after, "t2"), 60);
    assert_eq!(at(&after, "t2"), "c3.start+12f");
}

#[test]
fn attach_has_a_clip_and_what_is_linked_to_it_follow_and_detach_lets_go() {
    let project = run(
        &project(),
        "track add A2 audio\noverwrite media=m src=200f dur=12f at=c3.start+24f on=V2,A2 as=b,ba",
    );
    let script = "attach b to=c3";
    let applied = apply_script(&project, Catalog::builtin(), script).expect("runs");
    assert_eq!(
        print_ops(&project, Catalog::builtin(), &applied.ops).trim(),
        script
    );
    let after = applied.project;
    assert_eq!([at(&after, "b"), at(&after, "ba")], ["c3.start+24f"; 2]);
    assert_eq!([start(&after, "b"), start(&after, "ba")], [120; 2]);
    let script = "detach ba";
    let applied = apply_script(&after, Catalog::builtin(), script).expect("runs");
    assert_eq!(
        print_ops(&after, Catalog::builtin(), &applied.ops).trim(),
        script
    );
    assert_eq!(
        [at(&applied.project, "b"), at(&applied.project, "ba")],
        ["00:00:05:00"; 2]
    );
    // Not itself, not one that follows it, and not what it is linked to.
    assert!(refused(&after, "attach c3 to=c3").contains("`c3` cannot follow itself"));
    assert!(refused(&after, "attach c3 to=b").contains("`c3` cannot follow `b`, which follows it"));
    assert!(refused(&after, "attach ba to=b").contains("moves with it already"));
    assert!(refused(&after, "attach t2").contains("says which clip to follow"));
}

#[test]
fn a_follower_on_a_locked_track_is_left_where_it_is() {
    let locked = run(&project(), "track set V2 locked=true");
    assert!(refused(&locked, "attach t3 to=c1").contains("track.locked"));
    let moved = run(&locked, "move c3 by=+12f");
    assert_eq!([start(&moved, "c3"), start(&moved, "t3")], [108, 102]);
}

// ---- Random edits ------------------------------------------------------------------------

/// Four shots in a row; B-roll over the second that follows it, and a title that follows
/// the B-roll; music under all four that follows the first; and a clip over the third that
/// follows nothing. What an edit inserts plays another media, so that only the parts of one
/// shot play the same moments of a source.
const PROGRAM: &str = "kerf 1
project name=\"t\"
media m path=\"a.mov\" rate=24 dur=00:03:20:00 video=64x36
media n path=\"b.mov\" rate=24 dur=00:03:20:00 video=64x36
media mu path=\"music.wav\" rate=24 dur=00:03:20:00 audio=2ch@48000
sequence s1 rate=24 size=64x36 audio=48000
  track V1 video
    clip c1 media=m src=00:00:10:00 at=00:00:00:00 dur=00:00:02:00
    clip c2 media=m src=00:00:20:00 at=00:00:02:00 dur=00:00:02:00
    clip c3 media=m src=00:00:30:00 at=00:00:04:00 dur=00:00:02:00
    clip c4 media=m src=00:00:40:00 at=00:00:06:00 dur=00:00:02:00
  track V2 video
    clip b media=m src=00:01:00:00 at=c2.start+12f dur=00:00:01:00
  track V3 video
    clip t gen=title@1 at=b.start+6f dur=00:00:00:12
  track V4 video
    clip p gen=solid@1 at=00:00:04:12 dur=00:00:00:12
  track A1 audio
    clip mus media=mu src=00:00:00:00 at=c1.start dur=00:00:08:00
";

/// One edit, of a shot or a follower picked from those there are when it runs.
#[derive(Clone, Debug)]
enum Edit {
    Move(Index, i64),
    Slide(Index, i64),
    Trim {
        shot: Index,
        tail: bool,
        by: i64,
        ripple: bool,
    },
    Roll(Index, i64),
    Slip(Index, i64),
    Speed(Index, bool),
    Split(Index, i64),
    Delete(Index),
    RippleDelete(Index),
    Insert(Index, i64, i64),
    Overwrite(Index, i64, i64),
    /// Lift or extract a range, from every track or the shots' own.
    Range {
        shot: Index,
        from: i64,
        frames: i64,
        close: bool,
        every_track: bool,
    },
    Blade(Index, i64),
    Attach(Index, Index),
    Detach(Index),
    MoveFollower(Index, i64),
}

fn any_edit() -> impl Strategy<Value = Edit> {
    let shot = || any::<Index>();
    let by = || prop_oneof![-12i64..0, 1i64..13];
    let within = || 1i64..47;
    let frames = || 1i64..25;
    prop_oneof![
        (shot(), by()).prop_map(|(shot, by)| Edit::Move(shot, by)),
        (shot(), by()).prop_map(|(shot, by)| Edit::Slide(shot, by)),
        (shot(), any::<bool>(), by(), any::<bool>()).prop_map(|(shot, tail, by, ripple)| {
            Edit::Trim {
                shot,
                tail,
                by,
                ripple,
            }
        }),
        (shot(), by()).prop_map(|(shot, by)| Edit::Roll(shot, by)),
        (shot(), by()).prop_map(|(shot, by)| Edit::Slip(shot, by)),
        (shot(), any::<bool>()).prop_map(|(shot, fast)| Edit::Speed(shot, fast)),
        (shot(), within()).prop_map(|(shot, at)| Edit::Split(shot, at)),
        shot().prop_map(Edit::Delete),
        shot().prop_map(Edit::RippleDelete),
        (shot(), 0i64..48, frames()).prop_map(|(shot, at, frames)| Edit::Insert(shot, at, frames)),
        (shot(), 0i64..48, frames())
            .prop_map(|(shot, at, frames)| Edit::Overwrite(shot, at, frames)),
        (shot(), 0i64..48, frames(), any::<bool>(), any::<bool>()).prop_map(
            |(shot, from, frames, close, every_track)| Edit::Range {
                shot,
                from,
                frames,
                close,
                every_track,
            }
        ),
        (shot(), within()).prop_map(|(shot, at)| Edit::Blade(shot, at)),
        (shot(), shot()).prop_map(|(clip, to)| Edit::Attach(clip, to)),
        shot().prop_map(Edit::Detach),
        (shot(), by()).prop_map(|(clip, by)| Edit::MoveFollower(clip, by)),
    ]
}

/// The clips of a project's one sequence, track by track.
fn clips(project: &Project) -> impl Iterator<Item = &Clip> {
    project.sequences[0]
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
}

impl Edit {
    /// Its line on `project`, and the clip it is of: `None` where there is no such clip.
    fn line(&self, project: &Project) -> Option<(String, String)> {
        let shots: Vec<&str> = project.sequences[0].tracks[0]
            .clips
            .iter()
            .map(|clip| clip.id.as_str())
            .collect();
        let others: Vec<&str> = project.sequences[0].tracks[1..]
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.id.as_str())
            .collect();
        let all: Vec<&str> = clips(project).map(|clip| clip.id.as_str()).collect();
        let of = |index: &Index, from: &[&str]| {
            (!from.is_empty()).then(|| from[index.index(from.len())].to_owned())
        };
        let shot = |index: &Index| of(index, &shots);
        Some(match self {
            Self::Move(index, by) => {
                let shot = shot(index)?;
                (format!("move {shot} by={by:+}f"), shot)
            }
            Self::Slide(index, by) => {
                let shot = shot(index)?;
                (format!("slide {shot} {by:+}f"), shot)
            }
            Self::Trim {
                shot: index,
                tail,
                by,
                ripple,
            } => {
                let shot = shot(index)?;
                let (edge, ripple) = (
                    if *tail { "out" } else { "in" },
                    if *ripple { " ripple" } else { "" },
                );
                (format!("trim {shot} {edge} {by:+}f{ripple}"), shot)
            }
            Self::Roll(index, by) => {
                let shot = shot(index)?;
                (format!("roll {shot} {by:+}f"), shot)
            }
            Self::Slip(index, by) => {
                let shot = shot(index)?;
                (format!("slip {shot} {by:+}f"), shot)
            }
            Self::Speed(index, fast) => {
                let shot = shot(index)?;
                (
                    format!("speed {shot} {}", if *fast { "2" } else { "1/2" }),
                    shot,
                )
            }
            Self::Split(index, at) => {
                let shot = shot(index)?;
                (format!("split {shot} at={shot}.start+{at}f"), shot)
            }
            Self::Delete(index) => {
                let shot = shot(index)?;
                (format!("delete {shot}"), shot)
            }
            Self::RippleDelete(index) => {
                let shot = shot(index)?;
                (format!("ripple-delete {shot}"), shot)
            }
            Self::Insert(index, at, frames) => {
                let shot = shot(index)?;
                (
                    format!("insert media=n src=240f dur={frames}f at={shot}.start+{at}f on=V1"),
                    shot,
                )
            }
            Self::Overwrite(index, at, frames) => {
                let shot = shot(index)?;
                (
                    format!("overwrite media=n src=480f dur={frames}f at={shot}.start+{at}f on=V1"),
                    shot,
                )
            }
            Self::Range {
                shot: index,
                from,
                frames,
                close,
                every_track,
            } => {
                let shot = shot(index)?;
                let (verb, tracks) = (
                    if *close { "extract" } else { "lift" },
                    if *every_track { "" } else { " tracks=V1" },
                );
                (
                    format!(
                        "{verb} from={shot}.start+{from}f to={shot}.start+{}f{tracks}",
                        from + frames
                    ),
                    shot,
                )
            }
            Self::Blade(index, at) => {
                let shot = shot(index)?;
                (format!("blade at={shot}.start+{at}f"), shot)
            }
            Self::Attach(clip, to) => {
                let clip = of(clip, &others)?;
                (format!("attach {clip} to={}", of(to, &all)?), clip)
            }
            Self::Detach(clip) => {
                let clip = of(clip, &others)?;
                (format!("detach {clip}"), clip)
            }
            Self::MoveFollower(clip, by) => {
                let clip = of(clip, &others)?;
                (format!("move {clip} by={by:+}f"), clip)
            }
        })
    }

    /// Whether it cuts whatever is in a range or at a point, on every track.
    fn cuts_every_track(&self) -> bool {
        matches!(
            self,
            Self::Blade(..)
                | Self::Range {
                    every_track: true,
                    ..
                }
        )
    }
}

/// What a clip plays, and for how long: all that an edit that cuts it changes.
fn played(clip: &Clip) -> (Time, Option<(Time, Time)>) {
    (clip.duration, clip.source_range())
}

/// Hold one edit that ran, `line` of the clip `of`, to what a follower is.
fn hold(
    before: &Project,
    after: &Project,
    edit: &Edit,
    line: &str,
    of: &str,
) -> Result<(), TestCaseError> {
    // The document says it all: it reads back as the project it was written from.
    let document = printed(after);
    let read = parse_document(&document, Catalog::builtin())
        .map_err(|errors| TestCaseError::fail(format!("`{line}`: {errors:?}\n{document}")))?;
    prop_assert_eq!(&read.project, after, "`{}`\n{}", line, document);

    for follower in clips(before).filter(|clip| clip.follows.is_some()) {
        let id = follower.id.as_str();
        let followed = before
            .clip(follower.follows.as_ref().expect("a follower").as_str())
            .expect("the clip it follows");
        // A follower of a shot that a ripple delete removes goes with it when wholly over it.
        let taken_with_it = matches!(edit, Edit::RippleDelete(_))
            && followed.id.as_str() == of
            && follower.start >= followed.start
            && follower.end() <= followed.end();
        let Some(now) = after.clip(id) else {
            prop_assert!(
                id == of || edit.cuts_every_track() || taken_with_it,
                "`{}` removed `{}`",
                line,
                id
            );
            continue;
        };
        // No edit of another clip cuts it, but one that cuts every track.
        let whole = played(now) == played(follower);
        prop_assert!(
            whole || id == of || edit.cuts_every_track(),
            "`{}` cut `{}`",
            line,
            id
        );
        if id == of || !whole {
            continue;
        }
        // With the clip it followed gone, it is where it was.
        if after.clip(followed.id.as_str()).is_none() {
            prop_assert_eq!(now.start, follower.start, "`{}` moved `{}`", line, id);
            continue;
        }
        // A move or a slide of the clip it follows takes it as far.
        if matches!(
            edit,
            Edit::Move(..) | Edit::Slide(..) | Edit::MoveFollower(..)
        ) && followed.id.as_str() == of
        {
            let moved = after.clip(of).expect("the clip moved").start - followed.start;
            prop_assert_eq!(now.start - follower.start, moved, "`{}` and `{}`", line, id);
        }
        // It stands over the moment of the source it stood over, wherever a clip still plays
        // that moment, and follows that clip: the one it followed, or the part of it that
        // holds the moment now. A slip and a change of speed move what a clip shows under
        // what follows it, and are left out.
        let retimed =
            matches!(edit, Edit::Slip(..) | Edit::Speed(..)) && followed.id.as_str() == of;
        let over = followed.start <= follower.start && follower.start < followed.end();
        let (Some(moment), Content::Media { media, .. }, false, true) = (
            followed.source_at(follower.start),
            &followed.content,
            retimed,
            over,
        ) else {
            continue;
        };
        let plays_it: Vec<&Clip> = clips(after)
            .filter(|clip| {
                matches!(&clip.content, Content::Media { media: other, .. } if other == media)
                    && clip
                        .source_range()
                        .is_some_and(|(low, high)| low <= moment && moment < high)
            })
            .collect();
        // Shots trimmed and slipped far enough play the same moment twice: then none is it.
        // Nor is another clip that was there already and has come to play it, as the part
        // before a cut does when the cut is rolled later: the moment is its own now.
        let its_own =
            |clip: &Clip| clip.id == followed.id || before.clip(clip.id.as_str()).is_none();
        if let [plays_it] = plays_it[..]
            && its_own(plays_it)
        {
            prop_assert_eq!(
                now.follows.as_ref(),
                Some(&plays_it.id),
                "`{}` left `{}` following another than plays what it stood over",
                line,
                id
            );
            prop_assert_eq!(
                plays_it.played_at(moment),
                Some(now.start),
                "`{}` left `{}` off what it stood over",
                line,
                id
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Whatever edits run, one after another, a follower stays over what it stood over.
    #[test]
    fn random_edits_leave_each_follower_over_what_it_stood_over(
        edits in prop::collection::vec(any_edit(), 1..16),
    ) {
        let mut project = read(PROGRAM);
        for edit in &edits {
            let Some((line, of)) = edit.line(&project) else {
                continue;
            };
            // An edit the project refuses changes nothing.
            let Ok(applied) = apply_script(&project, Catalog::builtin(), &line) else {
                continue;
            };
            hold(&project, &applied.project, edit, &line, &of)?;
            project = applied.project;
        }
    }
}
