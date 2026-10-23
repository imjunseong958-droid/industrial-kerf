//! Edit semantics on a small sync scenario: picture on V1, its linked sound
//! on A1, and a music bed on A2.

use kerf_model::{
    apply, validate, AudioInfo, Catalog, Clip, ClipChange, Content, Edge, Id, Media, NewClip, Op,
    Param, Params, Project, Rate, Ratio, Sequence, Time, Track, TrackKind, Value, VideoInfo,
};
use kerf_model::{Align, TrackChange};

fn f(n: i64) -> Time {
    Rate::FPS_24.time_of(n)
}

fn id(text: &str) -> Id {
    Id::new(text).unwrap()
}

fn media_clip(
    clip: &str,
    media: &str,
    source: i64,
    start: i64,
    frames: i64,
    link: Option<&str>,
) -> Clip {
    let mut clip = Clip::new(
        id(clip),
        Content::Media {
            media: id(media),
            source_in: f(source),
            speed: Ratio::ONE,
        },
        f(start),
        f(frames),
    );
    clip.link = link.map(id);
    clip
}

fn track(name: &str, kind: TrackKind, clips: Vec<Clip>) -> Track {
    let mut track = Track::new(id(name), kind);
    track.clips = clips;
    track
}

/// V1: c1 [0,48) c2 [48,96) · A1: a1, a2 linked to them · A2: music [0,192)
fn scene() -> Project {
    let mut project = Project::new("test");
    project.media.push(Media {
        id: id("m1"),
        path: "camera.mov".into(),
        rate: Rate::FPS_24,
        drop_frame: false,
        start: Time::ZERO,
        duration: f(2400),
        video: Some(VideoInfo {
            width: 1920,
            height: 1080,
            alpha: false,
        }),
        audio: Some(AudioInfo {
            channels: 2,
            sample_rate: 48_000,
        }),
        reel: None,
    });
    project.media.push(Media {
        id: id("m2"),
        path: "music.wav".into(),
        rate: Rate::FPS_24,
        drop_frame: false,
        start: Time::ZERO,
        duration: f(7200),
        video: None,
        audio: Some(AudioInfo {
            channels: 2,
            sample_rate: 48_000,
        }),
        reel: None,
    });
    project.sequences.push(Sequence {
        id: id("s1"),
        name: "Main".into(),
        rate: Rate::FPS_24,
        drop_frame: false,
        width: 1920,
        height: 1080,
        sample_rate: 48_000,
        start: f(24 * 3600),
        tracks: vec![
            track(
                "V1",
                TrackKind::Video,
                vec![
                    media_clip("c1", "m1", 0, 0, 48, Some("L1")),
                    media_clip("c2", "m1", 240, 48, 48, Some("L2")),
                ],
            ),
            track(
                "A1",
                TrackKind::Audio,
                vec![
                    media_clip("a1", "m1", 0, 0, 48, Some("L1")),
                    media_clip("a2", "m1", 240, 48, 48, Some("L2")),
                ],
            ),
            track(
                "A2",
                TrackKind::Audio,
                vec![media_clip("music", "m2", 0, 0, 192, None)],
            ),
        ],
        markers: Vec::new(),
    });
    assert_eq!(validate(&project, Catalog::builtin()), vec![]);
    project
}

fn run(project: &Project, ops: Vec<Op>) -> Project {
    match apply(project, Catalog::builtin(), &ops) {
        Ok((project, _)) => project,
        Err(rejected) => panic!("rejected: {:?}", rejected),
    }
}

fn rejects(project: &Project, ops: Vec<Op>) -> &'static str {
    apply(project, Catalog::builtin(), &ops)
        .expect_err("expected a rejection")
        .diagnostics[0]
        .code
}

/// `(id, start, end)` in frames for each clip on a track.
fn layout(project: &Project, track: &str) -> Vec<(String, i64, i64)> {
    let rate = Rate::FPS_24;
    project.sequences[0]
        .track(track)
        .unwrap()
        .clips
        .iter()
        .map(|c| {
            (
                c.id.to_string(),
                rate.frames_exact(c.start).unwrap(),
                rate.frames_exact(c.end()).unwrap(),
            )
        })
        .collect()
}

fn source_in(project: &Project, clip: &str) -> i64 {
    Rate::FPS_24
        .frames_exact(project.clip(clip).unwrap().source_range().unwrap().0)
        .unwrap()
}

fn lock(track: &str) -> Op {
    Op::SetTrack {
        sequence: id("s1"),
        track: id(track),
        change: TrackChange {
            locked: Some(true),
            ..TrackChange::default()
        },
    }
}

fn new_media(source: i64) -> NewClip {
    NewClip::new(Content::Media {
        media: id("m1"),
        source_in: f(source),
        speed: Ratio::ONE,
    })
}

fn row(clip: &str, start: i64, end: i64) -> (String, i64, i64) {
    (clip.to_owned(), start, end)
}

#[test]
fn insert_opens_time_on_every_unlocked_track_and_links_targets() {
    let project = run(
        &scene(),
        vec![Op::Insert {
            sequence: id("s1"),
            at: f(48),
            duration: f(24),
            targets: vec![(id("V1"), new_media(480)), (id("A1"), new_media(480))],
        }],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 48), row("c3", 48, 72), row("c2", 72, 120)]
    );
    assert_eq!(
        layout(&project, "A1"),
        vec![row("a1", 0, 48), row("c4", 48, 72), row("a2", 72, 120)]
    );
    assert_eq!(
        layout(&project, "A2"),
        vec![row("music", 0, 48), row("c5", 72, 216)]
    );
    assert_eq!(
        project.clip("c3").unwrap().link,
        project.clip("c4").unwrap().link
    );
    assert_eq!(source_in(&project, "c5"), 48);
}

#[test]
fn overwrite_trims_and_splits_only_the_target_track() {
    let project = run(
        &scene(),
        vec![Op::Overwrite {
            sequence: id("s1"),
            at: f(36),
            duration: f(24),
            targets: vec![(id("V1"), new_media(1000))],
        }],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 36), row("c3", 36, 60), row("c2", 60, 96)]
    );
    assert_eq!(source_in(&project, "c2"), 252);
    assert_eq!(layout(&project, "A1"), layout(&scene(), "A1"));
}

#[test]
fn ripple_delete_refuses_to_desync_and_succeeds_when_the_bed_is_locked() {
    let scene = scene();
    assert_eq!(
        rejects(
            &scene,
            vec![Op::RippleDelete {
                clips: vec![id("c1")]
            }]
        ),
        "ripple.blocked"
    );
    let project = run(
        &scene,
        vec![
            lock("A2"),
            Op::RippleDelete {
                clips: vec![id("c1")],
            },
        ],
    );
    assert_eq!(layout(&project, "V1"), vec![row("c2", 0, 48)]);
    assert_eq!(layout(&project, "A1"), vec![row("a2", 0, 48)]);
    assert_eq!(layout(&project, "A2"), vec![row("music", 0, 192)]);
}

#[test]
fn extract_cuts_every_track_and_closes_the_range() {
    let project = run(
        &scene(),
        vec![Op::Extract {
            sequence: id("s1"),
            from: f(24),
            to: f(48),
            tracks: None,
        }],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 24), row("c2", 24, 72)]
    );
    assert_eq!(
        layout(&project, "A2"),
        vec![row("music", 0, 24), row("c3", 24, 168)]
    );
    assert_eq!(source_in(&project, "c3"), 48);
}

#[test]
fn trims_follow_links_and_respect_neighbors() {
    let scene = scene();
    let shorter = run(
        &scene,
        vec![Op::Trim {
            clip: id("c1"),
            edge: Edge::Out,
            delta: f(-12),
            ripple: false,
        }],
    );
    assert_eq!(layout(&shorter, "V1")[0], row("c1", 0, 36));
    assert_eq!(layout(&shorter, "A1")[0], row("a1", 0, 36));
    assert_eq!(
        rejects(
            &scene,
            vec![Op::Trim {
                clip: id("c1"),
                edge: Edge::Out,
                delta: f(12),
                ripple: false
            }]
        ),
        "trim.collision"
    );
    assert_eq!(
        rejects(
            &scene,
            vec![Op::Trim {
                clip: id("c1"),
                edge: Edge::In,
                delta: f(-12),
                ripple: false
            }]
        ),
        "trim.too_far"
    );
}

#[test]
fn ripple_trims_shift_downstream_material() {
    let scene = scene();
    let shorter = run(
        &scene,
        vec![
            lock("A2"),
            Op::Trim {
                clip: id("c1"),
                edge: Edge::Out,
                delta: f(-12),
                ripple: true,
            },
        ],
    );
    assert_eq!(
        layout(&shorter, "V1"),
        vec![row("c1", 0, 36), row("c2", 36, 84)]
    );
    assert_eq!(
        layout(&shorter, "A1"),
        vec![row("a1", 0, 36), row("a2", 36, 84)]
    );

    let longer = run(
        &scene,
        vec![Op::Trim {
            clip: id("c2"),
            edge: Edge::In,
            delta: f(-24),
            ripple: true,
        }],
    );
    assert_eq!(
        layout(&longer, "V1"),
        vec![row("c1", 0, 48), row("c2", 48, 120)]
    );
    assert_eq!(source_in(&longer, "c2"), 216);
    assert_eq!(
        layout(&longer, "A2"),
        vec![row("music", 0, 48), row("c3", 72, 216)]
    );
}

#[test]
fn roll_moves_the_cut_on_linked_tracks() {
    let project = run(
        &scene(),
        vec![Op::Roll {
            clip: id("c1"),
            delta: f(12),
        }],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 60), row("c2", 60, 96)]
    );
    assert_eq!(
        layout(&project, "A1"),
        vec![row("a1", 0, 60), row("a2", 60, 96)]
    );
    assert_eq!(source_in(&project, "c2"), 252);
    assert_eq!(
        rejects(
            &scene(),
            vec![Op::Roll {
                clip: id("c2"),
                delta: f(6)
            }]
        ),
        "roll.no_cut"
    );
}

#[test]
fn slip_changes_only_the_source() {
    let project = run(
        &scene(),
        vec![Op::Slip {
            clip: id("c2"),
            delta: f(24),
        }],
    );
    assert_eq!(layout(&project, "V1"), layout(&scene(), "V1"));
    assert_eq!(
        (source_in(&project, "c2"), source_in(&project, "a2")),
        (264, 264)
    );
    assert_eq!(
        rejects(
            &scene(),
            vec![Op::Slip {
                clip: id("c1"),
                delta: f(-1)
            }]
        ),
        "clip.source_range"
    );
}

#[test]
fn slide_adjusts_both_neighbors() {
    let mut scene = scene();
    let picture = &mut scene.sequences[0].tracks[0];
    for clip in &mut picture.clips {
        clip.link = None;
    }
    picture
        .clips
        .push(media_clip("c3", "m1", 600, 96, 48, None));
    for clip in &mut scene.sequences[0].tracks[1].clips {
        clip.link = None;
    }
    let project = run(
        &scene,
        vec![Op::Slide {
            clip: id("c2"),
            delta: f(12),
        }],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 60), row("c2", 60, 108), row("c3", 108, 144)]
    );
    assert_eq!(
        (source_in(&project, "c2"), source_in(&project, "c3")),
        (240, 612)
    );
}

#[test]
fn split_links_the_tails_of_linked_clips() {
    let project = run(
        &scene(),
        vec![Op::Split {
            clip: id("c1"),
            at: f(24),
            tail: Some(id("c1b")),
        }],
    );
    assert_eq!(
        layout(&project, "V1")[..2],
        [row("c1", 0, 24), row("c1b", 24, 48)]
    );
    assert_eq!(
        layout(&project, "A1")[..2],
        [row("a1", 0, 24), row("c3", 24, 48)]
    );
    let tail_link = project.clip("c1b").unwrap().link.clone();
    assert!(tail_link.is_some());
    assert_eq!(project.clip("c3").unwrap().link, tail_link);
    assert_ne!(project.clip("c1").unwrap().link, tail_link);
    assert_eq!(source_in(&project, "c1b"), 24);
}

#[test]
fn move_carries_linked_clips_and_refuses_overlap() {
    let project = run(
        &scene(),
        vec![Op::Move {
            clip: id("c2"),
            track: None,
            delta: f(48),
        }],
    );
    assert_eq!(layout(&project, "V1")[1], row("c2", 96, 144));
    assert_eq!(layout(&project, "A1")[1], row("a2", 96, 144));
    assert_eq!(
        rejects(
            &scene(),
            vec![Op::Move {
                clip: id("c2"),
                track: None,
                delta: f(-12)
            }]
        ),
        "move.collision"
    );
}

#[test]
fn set_validates_against_the_catalog() {
    let half = Param::Static(Value::Number(0.5));
    let project = run(
        &scene(),
        vec![Op::Set {
            clip: id("c1"),
            changes: vec![ClipChange::Param("opacity".into(), Some(half.clone()))],
        }],
    );
    assert_eq!(project.clip("c1").unwrap().props["opacity"], half);
    assert_eq!(
        rejects(
            &scene(),
            vec![Op::Set {
                clip: id("c1"),
                changes: vec![ClipChange::Param("gain".into(), Some(half))]
            }]
        ),
        "param.unknown"
    );
}

#[test]
fn speed_keeps_the_source_range() {
    let project = run(
        &scene(),
        vec![
            lock("A2"),
            Op::Speed {
                clip: id("c2"),
                speed: Ratio::parse("2").unwrap(),
            },
        ],
    );
    assert_eq!(layout(&project, "V1")[1], row("c2", 48, 72));
    let clip = project.clip("c2").unwrap();
    assert_eq!(clip.source_range(), Some((f(240), f(288))));
}

#[test]
fn transitions_need_handles_and_drop_when_their_cut_opens() {
    let dissolve = |from: &str, to: &str| Op::AddTransition {
        id: None,
        from: id(from),
        to: id(to),
        fx: id("dissolve"),
        duration: f(12),
        align: Align::Center,
        params: Params::new(),
    };
    let project = run(&scene(), vec![dissolve("c1", "c2")]);
    assert_eq!(project.sequences[0].tracks[0].transitions.len(), 1);

    let mut short = scene();
    short.sequences[0].tracks[0].clips[1] = media_clip("c2", "m1", 0, 48, 48, Some("L2"));
    short.sequences[0].tracks[1].clips[1] = media_clip("a2", "m1", 0, 48, 48, Some("L2"));
    let rejected = apply(&short, Catalog::builtin(), &[dissolve("c1", "c2")]).unwrap_err();
    assert_eq!(rejected.diagnostics[0].code, "transition.handles");

    let (opened, report) = apply(
        &project,
        Catalog::builtin(),
        &[Op::Insert {
            sequence: id("s1"),
            at: f(48),
            duration: f(24),
            targets: vec![(id("V1"), new_media(480))],
        }],
    )
    .unwrap();
    assert!(opened.sequences[0].tracks[0].transitions.is_empty());
    assert!(report.notes[0].contains("removed transition"));
}

#[test]
fn a_failing_operation_leaves_the_project_untouched() {
    let scene = scene();
    let rejected = apply(
        &scene,
        Catalog::builtin(),
        &[
            Op::Trim {
                clip: id("c1"),
                edge: Edge::Out,
                delta: f(-12),
                ripple: false,
            },
            Op::Delete {
                clips: vec![id("missing")],
            },
        ],
    )
    .unwrap_err();
    assert_eq!(rejected.op, Some(1));
    assert_eq!(layout(&scene, "V1")[0], row("c1", 0, 48));
}

#[test]
fn locked_tracks_refuse_edits() {
    assert_eq!(
        rejects(
            &scene(),
            vec![
                lock("V1"),
                Op::Delete {
                    clips: vec![id("c1")]
                }
            ]
        ),
        "track.locked"
    );
}

#[test]
fn delete_takes_linked_partners_and_keeps_gaps() {
    let (project, report) = apply(
        &scene(),
        Catalog::builtin(),
        &[Op::Delete {
            clips: vec![id("c1")],
        }],
    )
    .unwrap();
    assert_eq!(layout(&project, "V1"), vec![row("c2", 48, 96)]);
    assert_eq!(layout(&project, "A1"), vec![row("a2", 48, 96)]);
    assert_eq!(report.removed, vec![id("a1"), id("c1")]);
}

#[test]
fn validation_reports_broken_invariants() {
    let mut broken = scene();
    broken.sequences[0].tracks[0].clips[1].start = f(40);
    broken.sequences[0].tracks[1].clips[0].content = Content::Caption { text: "hi".into() };
    broken.sequences[0].tracks[2].clips[0].duration = Time::from_flicks(f(10).flicks() + 1);
    let codes: Vec<_> = validate(&broken, Catalog::builtin())
        .into_iter()
        .map(|d| d.code)
        .collect();
    for expected in ["clip.overlap", "clip.domain", "clip.misaligned"] {
        assert!(
            codes.contains(&expected),
            "{expected} missing from {codes:?}"
        );
    }
}

#[test]
fn deleted_ids_are_retired_and_never_reused() {
    let project = scene();
    let catalog = Catalog::builtin();
    let victim = project.sequences[0].tracks[0]
        .clips
        .last()
        .unwrap()
        .id
        .clone();
    let (after, _) = apply(
        &project,
        catalog,
        &[Op::Delete {
            clips: vec![victim.clone()],
        }],
    )
    .unwrap();
    assert!(after.retired.contains(&victim));
    let mut ids = after.id_gen();
    let prefix = victim
        .as_str()
        .trim_end_matches(|c: char| c.is_ascii_digit());
    assert_ne!(ids.next(prefix), victim);
    let mut reused = after.clone();
    reused.sequences[0].tracks[0].clips[0].id = victim.clone();
    assert!(kerf_model::validate(&reused, catalog)
        .iter()
        .any(|d| d.code == "id.retired"));
}
