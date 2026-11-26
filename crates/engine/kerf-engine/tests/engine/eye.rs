//! Where the eye is either side of a cut, read from the guides of the two shots.

use std::collections::BTreeMap;

use kerf_compose::{ComposeOptions, Compositor};
use kerf_engine::analysis::eye::{eye, file_picture, Cut, Eye, Going, Read, Seen, HELD};
use kerf_lang::parse_document;
use kerf_model::{Catalog, Media, Project, Rate};

/// How far off a share of the frame may be: under two pixels of a guide 180 high.
const NEAR: f64 = 0.01;

/// A shot's media and the steps that make it: its guide drawn from `sequence`, and the
/// clip `subject` of it handed to the model as where a thing goes.
fn shot(id: &str, line: &str, sequence: &str, drawn: &str, subject: &str) -> String {
    format!(
        "media {id} path=\"shots/{id}.mp4\" {line}\n  render {sequence} {{guide.mp4}}{drawn}\n  generate model=video prompt=\"a box\" guide={{guide.mp4}}{subject}\n"
    )
}

/// A guide's sequence: a backdrop, and over it a red box a fifth of the frame each way,
/// in its middle unless `set` says otherwise.
fn guide(id: &str, size: &str, subject: &str, set: &str) -> String {
    let scale = if set.contains("scale=") {
        ""
    } else {
        "scale=0.2 "
    };
    format!(
        concat!(
            "sequence {id} {size} audio=48000\n",
            "  track V1 video\n    clip back{id} gen=solid@1 at=0f dur=2s\n      set color=#203850\n",
            "  track V2 video\n    clip {subject} gen=solid@1 at=0f dur=2s\n      set color=#ff2010 {scale}{set}\n",
        ),
        id = id,
        size = size,
        subject = subject,
        scale = scale,
        set = set,
    )
}

fn project(media: &str, program: &str, guides: &str) -> Project {
    let text = format!(
        "kerf 1\nproject name=\"t\"\n{media}sequence main rate=24 size=320x180 audio=48000\n  track V1 video\n{program}{guides}"
    );
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}\n{text}"))
        .project
}

/// What a scene of two shots is made of: each shot's media line and what its `render`
/// step adds, the two clips of the program, and how each guide's box is set.
struct Scene<'a> {
    media: [&'a str; 2],
    drawn: [&'a str; 2],
    clips: &'a str,
    sizes: [&'a str; 2],
    boxes: [&'a str; 2],
}

const LINE: &str = "rate=24 dur=48f video=320x180";
const SIZE: &str = "rate=24 size=320x180";
const CLIPS: &str = "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 at=48f dur=48f\n";

impl Default for Scene<'_> {
    fn default() -> Self {
        Self {
            media: [LINE, LINE],
            drawn: ["", ""],
            clips: CLIPS,
            sizes: [SIZE, SIZE],
            boxes: ["", ""],
        }
    }
}

impl Scene<'_> {
    fn project(&self) -> Project {
        project(
            &format!(
                "{}{}",
                shot("m1", self.media[0], "g1", self.drawn[0], " moves=box1"),
                shot("m2", self.media[1], "g2", self.drawn[1], " moves=box2"),
            ),
            self.clips,
            &format!(
                "{}{}",
                guide("g1", self.sizes[0], "box1", self.boxes[0]),
                guide("g2", self.sizes[1], "box2", self.boxes[1]),
            ),
        )
    }
}

/// The cuts of the program, with no shot made yet.
fn cuts(project: &Project) -> Eye {
    eye(project, None, None, &BTreeMap::new(), &mut |_, _, _| {
        Ok(None)
    })
    .expect("reads")
}

/// The one cut of a scene, as read.
fn read(scene: &Scene) -> Read {
    let found = cuts(&scene.project());
    assert_eq!(found.cuts.len(), 1, "{found:?}");
    found.cuts[0]
        .read
        .clone()
        .unwrap_or_else(|why| panic!("{why}"))
}

/// Two boxes of the scene, where the second's is set by `second`.
fn seen(second: &str) -> Seen {
    read(&Scene {
        boxes: ["", second],
        ..Scene::default()
    })
    .guide
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < NEAR
}

fn boxes_near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| near(*a, b))
}

#[test]
fn a_subject_in_the_same_place_either_side_of_a_cut_does_not_jump() {
    let found = cuts(&Scene::default().project());
    let Cut {
        at,
        from,
        to,
        transition,
        under,
        read,
    } = &found.cuts[0];
    assert_eq!(
        (*at, from.as_str(), to.as_str()),
        (Rate::FPS_24.time_of(48), "c1", "c2")
    );
    assert!(
        transition.is_none() && under.is_empty() && found.unread == 0,
        "{found:?}"
    );
    let read = read.as_ref().unwrap();
    // What the eye is on is what each shot's `generate` step says goes somewhere.
    assert_eq!(read.on, ["box1", "box2"]);
    let guide = &read.guide;
    assert!(boxes_near(guide.out, [0.4, 0.4, 0.6, 0.6]), "{guide:?}");
    assert!(boxes_near(guide.into, [0.4, 0.4, 0.6, 0.6]), "{guide:?}");
    assert!(
        guide.jump < NEAR && near(guide.size, 1.0) && guide.held(),
        "{guide:?}"
    );
    // Nothing is made, so no take is read; and a box that holds still goes nowhere.
    assert!(read.take.is_none());
    assert_eq!(read.going, Some([Going::Still, Going::Still]));
    assert!(!read.reversed());
}

#[test]
fn a_subject_moved_by_three_tenths_jumps_three_tenths_of_the_frames_height() {
    // Down the frame, `y` is in frame heights.
    let lower = seen("y=0.3");
    assert!(
        near(lower.jump, 0.3) && near(lower.down, 0.3) && near(lower.across, 0.0),
        "{lower:?}"
    );
    assert!(boxes_near(lower.into, [0.4, 0.7, 0.6, 0.9]), "{lower:?}");
    assert!(!lower.held() && lower.jump > HELD);
    // Across it, `x` is in frame widths: three tenths of the height is 0.16875 of a
    // frame 16 by 9.
    let right = seen("x=0.16875");
    assert!(
        near(right.jump, 0.3) && near(right.across, 0.3) && near(right.down, 0.0),
        "{right:?}"
    );
    // The other way is the other sign, and both ways at once are one distance.
    let left = seen("x=-0.16875");
    assert!(near(left.across, -0.3) && near(left.jump, 0.3), "{left:?}");
    let both = seen("x=0.16875 y=-0.4");
    assert!(near(both.jump, 0.5) && near(both.down, -0.4), "{both:?}");
    // A jump under a tenth is held.
    assert!(seen("y=0.08").held());
}

#[test]
fn a_subject_half_again_as_large_after_the_cut_is_a_size_of_one_and_a_half() {
    let larger = seen("scale=0.3");
    assert!(near(larger.size, 1.5) && larger.jump < NEAR, "{larger:?}");
    assert!(near(seen("scale=0.1").size, 0.5));
}

/// The box goes from `from` to `to` of the frame's width over the guide's two seconds.
fn goes(from: f64, to: f64) -> String {
    format!("x={{0f:{from} 48f:{to}}}")
}

#[test]
fn a_subject_that_goes_right_and_then_left_is_reversed() {
    let (right, left) = (goes(-0.3, 0.3), goes(0.3, -0.3));
    let going = |first: &str, second: &str, clips: &str| {
        let read = read(&Scene {
            boxes: [first, second],
            clips,
            ..Scene::default()
        });
        (read.going.expect("both go somewhere"), read.reversed())
    };
    assert_eq!(
        going(&right, &left, CLIPS),
        ([Going::Right, Going::Left], true)
    );
    assert_eq!(
        going(&left, &right, CLIPS),
        ([Going::Left, Going::Right], true)
    );
    // The same way either side is no reversal, and neither is a subject that stops.
    assert_eq!(
        going(&right, &right, CLIPS),
        ([Going::Right, Going::Right], false)
    );
    assert_eq!(
        going(&right, "", CLIPS),
        ([Going::Right, Going::Still], false)
    );
    // The way a thing goes is the way it goes on the program's frame: a clip that is
    // flipped has it going the other way, and where it was to the right it is to the left.
    let flipped = "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 at=48f dur=48f\n      set scale_x=-1\n";
    assert_eq!(
        going(&right, &right, flipped),
        ([Going::Right, Going::Left], true)
    );
    let aside = read(&Scene {
        boxes: ["", "x=0.2"],
        clips: flipped,
        ..Scene::default()
    })
    .guide;
    assert!(boxes_near(aside.into, [0.2, 0.4, 0.4, 0.6]), "{aside:?}");
    // A clip one frame long says nothing of which way its subject goes.
    let short = "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 at=48f dur=1f\n";
    let read = read(&Scene {
        boxes: [&right, &left],
        clips: short,
        ..Scene::default()
    });
    assert!(read.going.is_none() && !read.reversed(), "{read:?}");
}

#[test]
fn a_guide_of_another_rate_size_or_shape_reads_the_same() {
    let plain = seen("x=0.1 y=0.2 scale=0.3");
    let same = |scene: Scene, what: &str| {
        let guide = read(&Scene {
            boxes: ["", "x=0.1 y=0.2 scale=0.3"],
            ..scene
        })
        .guide;
        assert!(
            boxes_near(guide.out, plain.out) && boxes_near(guide.into, plain.into),
            "{what}: {guide:?} beside {plain:?}"
        );
        assert!(
            near(guide.jump, plain.jump) && near(guide.size, plain.size),
            "{what}: {guide:?}"
        );
    };
    // The guide's sequence larger and at another rate than the program.
    same(
        Scene {
            sizes: [SIZE, "rate=30 size=640x360"],
            ..Scene::default()
        },
        "a larger sequence at 30 a second",
    );
    // The guide drawn at a model's size and rate, and the take made at them.
    same(
        Scene {
            media: [LINE, "rate=16 dur=32f video=160x90"],
            drawn: ["", " size=160x90 rate=16"],
            ..Scene::default()
        },
        "a guide at 160x90 and 16 a second",
    );
    // A take of another shape: the guide fills it, and its sides are cut off evenly. On
    // the program it is fitted by its height, so what is left of the picture is where
    // it was.
    same(
        Scene {
            media: [LINE, "rate=24 dur=48f video=240x180"],
            drawn: ["", " size=240x180"],
            ..Scene::default()
        },
        "a take 4 by 3, its sides cut off",
    );
    // And one cut off twice, by the guide's size and then by the take's.
    same(
        Scene {
            media: [
                "rate=24 dur=48f video=200x180",
                "rate=24 dur=48f video=240x180",
            ],
            drawn: [" size=280x180", " size=240x180"],
            ..Scene::default()
        },
        "both takes narrower than their guides",
    );
    // A take wider than the program is fitted by its width, smaller: its box is as much
    // smaller, about the frame's middle.
    let wide = read(&Scene {
        media: [LINE, "rate=24 dur=48f video=320x120"],
        drawn: ["", " size=320x120"],
        sizes: [SIZE, "rate=24 size=320x120"],
        boxes: ["", "x=0.25"],
        ..Scene::default()
    })
    .guide;
    assert!(
        boxes_near(wide.into, [0.65, 0.4333, 0.85, 0.5667]),
        "{wide:?}"
    );
    // A subject half in what a take's shape cuts off is what is left of it, as one at
    // the frame's edge is; one wholly in it is not in the take, and that is said.
    let narrow = |second: &str| {
        cuts(
            &Scene {
                media: [LINE, "rate=24 dur=48f video=240x180"],
                drawn: ["", " size=240x180"],
                boxes: ["", second],
                ..Scene::default()
            }
            .project(),
        )
        .cuts[0]
            .read
            .clone()
    };
    let left = narrow("x=0.3").unwrap().guide;
    assert!(boxes_near(left.into, [0.7, 0.4, 0.875, 0.6]), "{left:?}");
    let why = narrow("x=0.48").unwrap_err();
    assert!(
        why.contains("another shape") && why.contains("`box2`"),
        "{why}"
    );
}

#[test]
fn a_frame_of_a_take_is_a_moment_of_its_guide_and_not_a_frame_of_it() {
    // The box crosses the frame in the guide's two seconds, at 24 a second.
    let crosses = goes(-0.3, 0.3);
    let middle = |seen: [f64; 4]| (seen[0] + seen[2]) / 2.0;
    let at = |seconds: f64| 0.5 - 0.3 + 0.6 * seconds / 2.0;
    // A take at 12 a second, played to the cut: the program's last frame, the 48th, shows
    // the take's 24th, which is 23/12 of a second into the guide and not its 24th frame.
    let slow = read(&Scene {
        media: ["rate=12 dur=24f video=320x180", LINE],
        drawn: [" rate=12", ""],
        boxes: [&crosses, ""],
        ..Scene::default()
    })
    .guide;
    assert!(near(middle(slow.out), at(23.0 / 12.0)), "{slow:?}");
    assert!(!near(middle(slow.out), at(23.0 / 24.0)));
    // A clip trimmed into its take starts on the moment it is trimmed to.
    let trimmed =
        "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 src=12f at=48f dur=24f\n";
    let later = read(&Scene {
        boxes: ["", &crosses],
        clips: trimmed,
        ..Scene::default()
    })
    .guide;
    assert!(near(middle(later.into), at(0.5)), "{later:?}");
    // One played at twice the speed ends twice as far in, and one played backward starts
    // on its last frame and goes the other way.
    let fast = "    clip c1 media=m1 at=0f dur=20f speed=2\n    clip c2 media=m2 at=20f dur=48f\n";
    let sped = read(&Scene {
        boxes: [&crosses, ""],
        clips: fast,
        ..Scene::default()
    });
    assert!(near(middle(sped.guide.out), at(38.0 / 24.0)), "{sped:?}");
    let back = "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 at=48f dur=48f speed=-1\n";
    let backward = read(&Scene {
        boxes: [&crosses, &crosses],
        clips: back,
        ..Scene::default()
    });
    assert!(
        near(middle(backward.guide.into), at(47.0 / 24.0)),
        "{backward:?}"
    );
    assert_eq!(backward.going, Some([Going::Right, Going::Left]));
    // A guide drawn from a second into its sequence starts there.
    let part = read(&Scene {
        media: [LINE, "rate=24 dur=24f video=320x180"],
        drawn: ["", " from=1"],
        clips: "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 at=48f dur=24f\n",
        boxes: ["", &crosses],
        ..Scene::default()
    })
    .guide;
    assert!(near(middle(part.into), at(1.0)), "{part:?}");
    // A clip that plays its take past where the guide ends says so.
    let long = "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 src=60f at=48f dur=12f\n";
    let past = cuts(
        &Scene {
            media: [LINE, "rate=24 dur=96f video=320x180"],
            clips: long,
            ..Scene::default()
        }
        .project(),
    );
    let why = past.cuts[0].read.as_ref().unwrap_err();
    assert!(why.contains("past where its guide `g2` ends"), "{why}");
}

/// The cut of a scene whose second clip is set by `set`, lines of the document under it.
fn with_second_clip(set: &str, second_box: &str) -> std::result::Result<Read, String> {
    let clips =
        format!("    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 at=48f dur=48f\n{set}");
    let found = cuts(
        &Scene {
            boxes: ["", second_box],
            clips: &clips,
            ..Scene::default()
        }
        .project(),
    );
    found.cuts[0].read.clone()
}

#[test]
fn a_clip_made_smaller_and_moved_on_the_program_is_read_where_it_is_put() {
    // Half the size about the frame's middle, then a quarter of the frame right and up.
    let moved = with_second_clip("      set scale=0.5 x=0.25 y=-0.25\n", "")
        .unwrap()
        .guide;
    assert!(
        boxes_near(moved.into, [0.70, 0.20, 0.80, 0.30]),
        "{moved:?}"
    );
    assert!(
        near(moved.size, 0.5) && near(moved.across, 0.25 * 16.0 / 9.0),
        "{moved:?}"
    );
    assert!(near(moved.down, -0.25), "{moved:?}");
    // Turned a quarter round about the frame's middle, a box 32 pixels to its right is
    // 32 below it, and as wide as it was tall: 36 by 64 of the frame's 320 by 180.
    let turned = with_second_clip("      set rotation=90\n", "x=0.1")
        .unwrap()
        .guide;
    assert!(
        boxes_near(turned.into, [0.44375, 0.5, 0.55625, 0.8556]),
        "{turned:?}"
    );
    // Moved with keyframes, it is where the keys have it on the frame it starts.
    let keyed = with_second_clip("      set x={0f:0.2 48f:-0.2}\n", "").unwrap();
    assert!(
        boxes_near(keyed.guide.into, [0.6, 0.4, 0.8, 0.6]),
        "{keyed:?}"
    );
    assert_eq!(keyed.going, Some([Going::Still, Going::Left]));
    // With a parent that moves it, it is where the parent takes it.
    let parented = format!(
        "{}{}",
        "    clip c1 media=m1 at=0f dur=48f\n    clip c2 media=m2 at=48f dur=48f parent=n1\n",
        "  track V2 video\n    clip n1 gen=null@1 at=48f dur=48f\n      set y=0.2\n",
    );
    let led = read(&Scene {
        clips: &parented,
        ..Scene::default()
    })
    .guide;
    assert!(boxes_near(led.into, [0.4, 0.6, 0.6, 0.8]), "{led:?}");
    // A crop that leaves the box whole moves it as the picture is fitted again: with
    // three tenths off its left, what is left is in the frame's middle.
    let cropped = with_second_clip("      set crop_l=0.3\n", "")
        .unwrap()
        .guide;
    assert!(
        boxes_near(cropped.into, [0.25, 0.4, 0.45, 0.6]),
        "{cropped:?}"
    );
    // What moves the box off the frame, or cuts it, is said and no number given.
    let gone = with_second_clip("      set x=1.5\n", "").unwrap_err();
    assert!(gone.contains("`c2` puts `box2` out of the frame"), "{gone}");
    let cut = with_second_clip("      set crop_l=0.45\n", "").unwrap_err();
    assert!(cut.contains("the crop of `c2` cuts `box2`"), "{cut}");
    let masked = with_second_clip("      mask k1 rect=0.1,0.1,0.9,0.9\n", "").unwrap_err();
    assert!(masked.contains("`c2` has a mask"), "{masked}");
    let deep = with_second_clip("      set three_d=true z=1\n", "").unwrap_err();
    assert!(deep.contains("`c2`") && deep.contains("in depth"), "{deep}");
    let clear = with_second_clip("      set opacity=0\n", "").unwrap_err();
    assert!(clear.contains("`c2` is clear where it starts"), "{clear}");
    // An effect that may move the picture is named; a grade is read through.
    let flipped = with_second_clip("      fx distort.transform@2\n", "x=0.2").unwrap_err();
    assert!(
        flipped.contains("`c2` has `distort.transform` on it"),
        "{flipped}"
    );
    let graded = with_second_clip("      fx color.basic@1 exposure=0.3\n", "x=0.2")
        .unwrap()
        .guide;
    assert!(boxes_near(graded.into, [0.6, 0.4, 0.8, 0.6]), "{graded:?}");
    // A look is the keys and effects it stands for. One that pushes in has the box as
    // much larger as the clip ends; one that shakes blurs as it goes, and is read through.
    let pushed = "    clip c1 media=m1 at=0f dur=48f\n      fx look.slow-push@1 amount=0.5 length=47f\n    clip c2 media=m2 at=48f dur=48f\n      fx look.shake@1 hit=24f\n";
    let look = read(&Scene {
        clips: pushed,
        ..Scene::default()
    })
    .guide;
    assert!(boxes_near(look.out, [0.35, 0.35, 0.65, 0.65]), "{look:?}");
    assert!(
        boxes_near(look.into, [0.4, 0.4, 0.6, 0.6]) && near(look.size, 1.0 / 1.5),
        "{look:?}"
    );
}

#[test]
fn a_cut_that_cannot_be_read_says_why() {
    let why = |media: &str, program: &str, subjects: &[(&str, &str)]| {
        let project = project(
            media,
            program,
            &format!(
                "{}{}",
                guide("g1", SIZE, "box1", ""),
                guide("g2", SIZE, "box2", "")
            ),
        );
        let subjects = subjects
            .iter()
            .map(|(media, clip)| ((*media).to_owned(), (*clip).to_owned()))
            .collect();
        let found = eye(&project, None, None, &subjects, &mut |_, _, _| Ok(None)).unwrap();
        assert_eq!(found.cuts.len(), 1, "{found:?}");
        found.cuts[0].read.clone().map(|read| read.on)
    };
    let one = shot("m1", LINE, "g1", "", " moves=box1");
    // Footage nothing made has no guide.
    let footage = format!("{one}media m2 path=\"walk.mp4\" {LINE}\n");
    assert_eq!(
        why(&footage, CLIPS, &[]).unwrap_err(),
        "`c2` plays footage with no guide"
    );
    // A shot whose steps draw nothing has none either.
    let undrawn = format!(
        "{one}media m2 path=\"shots/m2.mp4\" {LINE}\n  generate model=video prompt=\"a box\"\n"
    );
    assert!(why(&undrawn, CLIPS, &[])
        .unwrap_err()
        .contains("no `render` step"));
    // A guide with no subject named: the call names it, and then it is read.
    let unnamed = format!("{one}{}", shot("m2", LINE, "g2", "", ""));
    let unknown = why(&unnamed, CLIPS, &[]).unwrap_err();
    assert!(
        unknown.contains("nothing says what the eye is on in `m2`")
            && unknown.contains("`moves=<clip>`")
            && unknown.contains("`subjects`"),
        "{unknown}"
    );
    assert_eq!(
        why(&unnamed, CLIPS, &[("m2", "box2")]).unwrap(),
        ["box1", "box2"]
    );
    // What the call names is taken before what the steps say.
    let both = format!("{one}{}", shot("m2", LINE, "g2", "", " moves=box2"));
    assert_eq!(
        why(&both, CLIPS, &[("m2", "backg2")]).unwrap(),
        ["box1", "backg2"]
    );
    // A subject that is no clip, one of a sequence the shot's guide is not drawn from,
    // and one that draws nothing.
    let slip = why(&unnamed, CLIPS, &[("m2", "box3")]).unwrap_err();
    assert!(
        slip.contains("the subject of `m2`: no clip `box3`; did you mean"),
        "{slip}"
    );
    let elsewhere = why(&unnamed, CLIPS, &[("m2", "box1")]).unwrap_err();
    assert_eq!(
        elsewhere,
        "`box1` is a clip of `g1`, which no `render` step of `m2` draws"
    );
    // A clip that is no shot: a title, and a sequence nested.
    let title = "    clip c1 media=m1 at=0f dur=48f\n    clip t1 gen=title@1 at=48f dur=48f\n      set text=\"End\"\n";
    assert_eq!(
        why(&both, title, &[]).unwrap_err(),
        "`t1` is a `gen=title` clip, not a shot with a guide"
    );
    let nested = "    clip c1 media=m1 at=0f dur=48f\n    clip n1 seq=g2 at=48f dur=48f\n";
    assert!(why(&both, nested, &[])
        .unwrap_err()
        .contains("`n1` plays the sequence `g2`"));
}

#[test]
fn a_subject_the_guide_does_not_show_at_the_cut_is_said() {
    // Off the frame as the second shot starts, and on it a second later.
    let found = cuts(
        &Scene {
            boxes: ["", "x={0f:2 24f:2 48f:0}"],
            ..Scene::default()
        }
        .project(),
    );
    let why = found.cuts[0].read.as_ref().unwrap_err();
    assert_eq!(why, "the guide `g2` does not show `box2` where `c2` starts");
    // On the frame at the cut and off it a quarter second before: read, with no way said.
    let read = read(&Scene {
        boxes: ["x={0f:2 44f:2 47f:0}", ""],
        ..Scene::default()
    });
    assert!(read.going.is_none(), "{read:?}");
}

#[test]
fn only_where_one_clip_ends_as_the_next_starts_is_a_cut() {
    let media = format!(
        "{}{}",
        shot(
            "m1",
            "rate=24 dur=96f video=320x180",
            "g1",
            "",
            " moves=box1"
        ),
        shot("m2", LINE, "g2", "", " moves=box2"),
    );
    let guides = format!(
        "{}{}",
        guide("g1", SIZE, "box1", "").replace("dur=2s", "dur=4s"),
        guide("g2", SIZE, "box2", "x=0.2"),
    );
    // On the first track: a split that plays on, a cut, a gap, a cut under a transition,
    // and a clip turned off. On the second: a title over the first cut, and a cut of its own.
    let program = concat!(
        "    clip c1 media=m1 at=0f dur=24f\n",
        "    clip c1b media=m1 src=24f at=24f dur=24f\n",
        "    clip c2 media=m2 at=48f dur=24f\n",
        "    clip c3 media=m1 at=96f dur=24f\n",
        "    clip c4 media=m2 src=6f at=120f dur=24f\n",
        "    clip c5 media=m1 at=144f dur=24f enabled=false\n",
        "    transition x1 from=c3 to=c4 fx=dissolve@1 dur=6f\n",
        "  track V2 video\n",
        "    clip t1 gen=title@1 at=40f dur=20f\n      set text=\"Over\"\n",
        "    clip cam gen=camera@1 at=60f dur=30f\n",
        "    clip u1 media=m2 at=90f dur=20f\n",
        "    clip u2 media=m1 at=110f dur=20f\n",
    );
    let project = project(&media, program, &guides);
    let found = cuts(&project);
    let said: Vec<(i64, &str, &str)> = found
        .cuts
        .iter()
        .map(|cut| {
            (
                Rate::FPS_24.frames_round(cut.at),
                cut.from.as_str(),
                cut.to.as_str(),
            )
        })
        .collect();
    // In the order they play, whichever track they are on.
    assert_eq!(
        said,
        [(48, "c1b", "c2"), (110, "u1", "u2"), (120, "c3", "c4")],
        "{found:?}"
    );
    // What draws on a track above is named, and a camera is not; a transition is named.
    assert_eq!(found.cuts[0].under, ["t1"]);
    assert!(found.cuts[1].under.is_empty());
    assert_eq!(found.cuts[2].under, ["u2"]);
    assert_eq!(found.cuts[2].transition.as_deref(), Some("dissolve"));
    assert!(found.cuts.iter().all(|cut| cut.read.is_ok()), "{found:?}");
    // Within a range, the cuts that start in it.
    let range = (Rate::FPS_24.time_of(100), Rate::FPS_24.time_of(120));
    let some = eye(
        &project,
        None,
        Some(range),
        &BTreeMap::new(),
        &mut |_, _, _| Ok(None),
    )
    .unwrap();
    assert_eq!(some.cuts.len(), 1);
    assert_eq!(some.cuts[0].from, "u1");
    // A sequence other than the first is read by its name, and one that is not there is refused.
    assert!(eye(
        &project,
        Some("g1"),
        None,
        &BTreeMap::new(),
        &mut |_, _, _| Ok(None)
    )
    .unwrap()
    .cuts
    .is_empty());
    assert!(eye(
        &project,
        Some("none"),
        None,
        &BTreeMap::new(),
        &mut |_, _, _| Ok(None)
    )
    .is_err());
}

/// The picture a take would have were it its guide's own: frame `frame` of `media`, from
/// the sequence its `render` step draws in `project`, filling `size` with what is over
/// cut off evenly.
fn guides_own_picture(
    project: &Project,
    media: &Media,
    frame: i64,
    (width, height): (u32, u32),
) -> Vec<u8> {
    let sequence = match media.id.as_str() {
        "m1" => "g1",
        _ => "g2",
    };
    let drawn = Compositor::new(
        project,
        Catalog::builtin(),
        &ComposeOptions {
            sequence: Some(sequence.to_owned()),
            ..ComposeOptions::default()
        },
    )
    .expect("a guide");
    let picture = drawn.frame(media.rate.time_of(frame), 1.0);
    let (wide, tall) = (picture.width as usize, picture.height as usize);
    let (w, h) = (width as usize, height as usize);
    assert!(w <= wide && h <= tall, "{w}x{h} of {wide}x{tall}");
    let (left, top) = ((wide - w) / 2, (tall - h) / 2);
    (top..top + h)
        .flat_map(|y| &picture.rgba[(y * wide + left) * 4..(y * wide + left + w) * 4])
        .copied()
        .collect()
}

/// The one cut of `scene`, its takes being the pictures `takes` draws.
fn read_with_takes(scene: &Scene, takes: &Project) -> Read {
    let project = scene.project();
    let found = eye(
        &project,
        None,
        None,
        &BTreeMap::new(),
        &mut |media, frame, size| Ok(Some(guides_own_picture(takes, media, frame, size))),
    )
    .expect("reads");
    found.cuts[0]
        .read
        .clone()
        .unwrap_or_else(|why| panic!("{why}"))
}

#[test]
fn a_take_that_is_its_guides_own_picture_reads_as_the_guide_does() {
    let scene = Scene {
        boxes: ["x=-0.1", "x=0.15 y=0.1 scale=0.3"],
        ..Scene::default()
    };
    let read = read_with_takes(&scene, &scene.project());
    let take = read
        .take
        .clone()
        .expect("the shots have files")
        .expect("a take");
    // The guide's own picture is followed as the take is, so the same picture is where
    // the guide is, whatever a tracker's box adds to an ids pass's.
    assert!(
        boxes_near(take.out, read.guide.out) && boxes_near(take.into, read.guide.into),
        "{read:?}"
    );
    assert!(
        near(take.jump, read.guide.jump) && near(take.size, read.guide.size),
        "{read:?}"
    );
    // The same of takes of another shape than their guides.
    let narrow = Scene {
        media: [
            "rate=24 dur=48f video=240x180",
            "rate=24 dur=48f video=240x180",
        ],
        drawn: [" size=240x180", " size=240x180"],
        ..scene
    };
    let read = read_with_takes(&narrow, &narrow.project());
    let take = read.take.clone().unwrap().unwrap();
    assert!(
        boxes_near(take.out, read.guide.out) && boxes_near(take.into, read.guide.into),
        "{read:?}"
    );
}

#[test]
fn a_take_with_its_subject_elsewhere_than_its_guide_reads_apart_from_it() {
    // The guides have the box in one place either side of the cut. The second take has
    // it a twentieth of the frame's width to the right, and larger.
    let asked = Scene::default();
    let made = Scene {
        boxes: ["", "x=0.05 scale=0.25"],
        ..Scene::default()
    };
    let read = read_with_takes(&asked, &made.project());
    let take = read.take.clone().unwrap().unwrap();
    assert!(read.guide.held() && read.guide.jump < NEAR, "{read:?}");
    // The first take is its guide's; the second is off by what it was made off by.
    assert!(boxes_near(take.out, read.guide.out), "{read:?}");
    assert!((take.across - 0.05 * 16.0 / 9.0).abs() < 0.02, "{read:?}");
    assert!((take.size - 1.25).abs() < 0.1, "{read:?}");
    assert!(take.down.abs() < 0.02, "{read:?}");
}

#[test]
fn a_take_that_is_not_there_or_cannot_be_read_is_told_from_one_that_is() {
    let scene = Scene::default();
    let project = scene.project();
    let read = |takes: &mut kerf_engine::analysis::eye::Takes| {
        eye(&project, None, None, &BTreeMap::new(), takes)
            .unwrap()
            .cuts[0]
            .read
            .clone()
            .unwrap()
    };
    // One shot made and the other not: no take is read, and the guide still is.
    let one = read(&mut |media, frame, size| {
        Ok((media.id.as_str() == "m1").then(|| guides_own_picture(&project, media, frame, size)))
    });
    assert!(one.take.is_none() && one.guide.held(), "{one:?}");
    // A file that cannot be read says so, in place of a reading.
    let broken = read(&mut |media, _, _| Err(format!("`{}`: not a picture", media.id)));
    assert_eq!(broken.take, Some(Err("`m1`: not a picture".to_owned())));
    // A take that is one flat color has nothing to follow where the guide has the box:
    // the speck a tracker cuts out of it is not taken for the box, in the same place.
    let flat = read(&mut |_, _, (width, height)| {
        Ok(Some([40u8, 60, 80, 255].repeat((width * height) as usize)))
    });
    let why = flat.take.clone().unwrap().unwrap_err();
    assert_eq!(why, "`m1` has nothing to follow where the guide has `box1`");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test runs the programs it checks, and git or FFmpeg to set itself up, and waits on each"
)]
fn a_frame_of_a_take_is_read_from_its_file_by_its_number() {
    // A white box 16 across on black, 160 by 90 at 24 a second, four pixels further right
    // on each frame: which frame a picture is of is where its box is.
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("take.mp4");
    let made = std::process::Command::new(kerf_engine::process::ffmpeg::resolve_ffmpeg_bin())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("color=c=black:s=160x90:r=24:d=1")
        .args(["-f", "lavfi", "-i", "color=c=white:s=16x16:r=24:d=1"])
        .args(["-filter_complex", "[0:v][1:v]overlay=x=4*n:y=30"])
        .args(["-c:v", "libx264", "-g", "12", "-pix_fmt", "yuv420p"])
        .arg(&file)
        .output()
        .expect("ffmpeg runs");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let media = |path: &std::path::Path| {
        let text = format!(
            "kerf 1\nproject name=\"t\"\nmedia m1 path=\"{}\" rate=24 dur=24f video=160x90\nsequence s1 rate=24 size=160x90 audio=48000\n",
            path.display()
        );
        parse_document(&text, Catalog::builtin())
            .unwrap()
            .project
            .media
            .remove(0)
    };
    let take = media(&file);
    // Where the box is on a frame, counted by decoding the file from its start.
    let counted = |frame: i64| -> i64 {
        let read = std::process::Command::new(kerf_engine::process::ffmpeg::resolve_ffmpeg_bin())
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&file)
            .args(["-vf", &format!("select=eq(n\\,{frame}),format=gray")])
            .args(["-frames:v", "1", "-f", "rawvideo", "-"])
            .output()
            .expect("ffmpeg runs");
        let row = &read.stdout[40 * 160..41 * 160];
        row.iter().position(|grey| *grey > 128).expect("a box") as i64
    };
    let left_of = |picture: &[u8], width: usize, row: usize| -> i64 {
        let row = &picture[row * width * 4..(row + 1) * width * 4];
        row.as_chunks::<4>()
            .0
            .iter()
            .position(|pixel| pixel[0] > 128)
            .expect("a box") as i64
    };
    // Its first frame, its second, one past a keyframe, and its last: each is the frame
    // of that number, and no other, each being four pixels on from the one before.
    let mut before: Option<(i64, i64)> = None;
    for frame in [0i64, 1, 13, 23] {
        let picture = file_picture(&take, frame, (160, 90))
            .unwrap()
            .expect("the file is there");
        assert_eq!(picture.len(), 160 * 90 * 4);
        let left = left_of(&picture, 160, 40);
        assert!(
            (left - counted(frame)).abs() <= 1,
            "frame {frame}: its box is at {left}"
        );
        if let Some((earlier, was)) = before {
            assert!(
                (left - was - 4 * (frame - earlier)).abs() <= 1,
                "frame {frame}: {left} after {was}"
            );
        }
        before = Some((frame, left));
    }
    // At half the size, it is the same frame smaller.
    let small = file_picture(&take, 13, (80, 45)).unwrap().unwrap();
    assert!((left_of(&small, 80, 20) * 2 - counted(13)).abs() <= 2);
    // A shot not made has no file, which is no error; a file that is no picture is one.
    assert_eq!(
        file_picture(&media(&folder.path().join("none.mp4")), 0, (160, 90)),
        Ok(None)
    );
    let broken = folder.path().join("broken.mp4");
    std::fs::write(&broken, "not a picture").unwrap();
    let why = file_picture(&media(&broken), 0, (160, 90)).unwrap_err();
    assert!(why.starts_with("`m1`: "), "{why}");
}
