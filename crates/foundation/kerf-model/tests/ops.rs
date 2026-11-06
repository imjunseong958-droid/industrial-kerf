//! Edit semantics on a small sync scenario: picture on V1, its linked sound
//! on A1, and a music bed on A2.

use kerf_model::time::{format_timecode, parse_timecode};
use kerf_model::{
    apply, validate, AudioInfo, Catalog, Clip, ClipChange, Content, Edge, Id, Media, NewClip, Op,
    Param, Params, Project, Rate, Ratio, Sequence, Time, Track, TrackKind, Value, VideoInfo,
};
use kerf_model::{Align, SequenceChange, TrackChange, TrackRoleChange};

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
            speed: Ratio::ONE.into(),
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
            ..Default::default()
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
        buses: Vec::new(),
        id: id("s1"),
        name: "Main".into(),
        rate: Rate::FPS_24,
        drop_frame: false,
        width: 1920,
        height: 1080,
        sample_rate: 48_000,
        color: Default::default(),
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
        speed: Ratio::ONE.into(),
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
fn a_title_generator_inserts_like_media_and_only_on_picture_tracks() {
    let mut params = Params::new();
    params.insert("text".into(), Param::Static(Value::Text("Day One".into())));
    let title = || {
        NewClip::new(Content::Generator {
            fx: id("title"),
            version: 1,
            params: params.clone(),
        })
    };
    let project = run(
        &scene(),
        vec![Op::Insert {
            sequence: id("s1"),
            at: f(48),
            duration: f(24),
            targets: vec![(id("V1"), title())],
        }],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 48), row("c3", 48, 72), row("c2", 72, 120)]
    );
    let Content::Generator { fx, params, .. } = &project.clip("c3").unwrap().content else {
        panic!("c3 is a generator");
    };
    assert_eq!(fx.as_str(), "title");
    assert_eq!(params["text"], Param::Static(Value::Text("Day One".into())));
    assert_eq!(validate(&project, Catalog::builtin()), vec![]);
    // A picture generator has no place on a sound track.
    assert!(apply(
        &scene(),
        Catalog::builtin(),
        &[Op::Overwrite {
            sequence: id("s1"),
            at: f(0),
            duration: f(24),
            targets: vec![(id("A2"), title())],
        }],
    )
    .is_err());
}

#[test]
fn the_frame_rate_changes_only_while_the_sequence_is_empty_and_keeps_the_start() {
    let set_rate = |rate: Rate| Op::SetSequence {
        sequence: id("s1"),
        change: SequenceChange {
            rate: Some(rate),
            ..SequenceChange::default()
        },
    };
    assert_eq!(
        rejects(&scene(), vec![set_rate(Rate::FPS_30)]),
        "sequence.conform"
    );
    // The same rate is no change at all.
    run(&scene(), vec![set_rate(Rate::FPS_24)]);
    let mut empty = scene();
    for track in &mut empty.sequences[0].tracks {
        track.clips.clear();
    }
    // 01:00:00:00 at 24 fps reads 01:00:00:00 at 29.97 too.
    let project = run(&empty, vec![set_rate(Rate::FPS_29_97)]);
    let sequence = &project.sequences[0];
    assert_eq!(sequence.rate, Rate::FPS_29_97);
    assert_eq!(
        Rate::FPS_29_97.frames_exact(sequence.start),
        Some(30 * 3600)
    );
    assert!(!sequence.drop_frame);
    assert_eq!(validate(&project, Catalog::builtin()), vec![]);
    // A drop-frame start stays drop-frame only where the rate has it.
    let mut drop = project.clone();
    (drop.sequences[0].start, drop.sequences[0].drop_frame) =
        parse_timecode("01:00:00;00", Rate::FPS_29_97).unwrap();
    let at_30 = run(&drop, vec![set_rate(Rate::FPS_30)]);
    assert!(!at_30.sequences[0].drop_frame);
    assert_eq!(
        format_timecode(at_30.sequences[0].start, Rate::FPS_30, false),
        "01:00:00:00"
    );
    assert_eq!(validate(&at_30, Catalog::builtin()), vec![]);
    let fps_59 = Rate::new(60_000, 1001).unwrap();
    let at_59 = run(&drop, vec![set_rate(fps_59)]);
    assert!(at_59.sequences[0].drop_frame);
    assert_eq!(
        format_timecode(at_59.sequences[0].start, fps_59, true),
        "01:00:00;00"
    );
    assert_eq!(validate(&at_59, Catalog::builtin()), vec![]);
    // A start given with the rate is taken as it is.
    let given = run(
        &empty,
        vec![Op::SetSequence {
            sequence: id("s1"),
            change: SequenceChange {
                rate: Some(Rate::FPS_25),
                start: Some((Time::ZERO, false)),
                ..SequenceChange::default()
            },
        }],
    );
    assert_eq!(given.sequences[0].start, Time::ZERO);
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

fn sync(track: &str, on: bool) -> Op {
    Op::SetTrackRole {
        sequence: id("s1"),
        track: id(track),
        change: TrackRoleChange {
            sync_locked: Some(on),
            ..TrackRoleChange::default()
        },
    }
}

#[test]
fn ripple_edits_leave_tracks_that_are_not_sync_locked_where_they_are() {
    let scene = scene();
    assert!(scene.sequences[0].tracks.iter().all(|t| t.sync_locked));

    // Ripple delete: the bed is not cut, and does not move.
    let project = run(
        &scene,
        vec![
            sync("A2", false),
            Op::RippleDelete {
                clips: vec![id("c1")],
            },
        ],
    );
    assert_eq!(layout(&project, "V1"), vec![row("c2", 0, 48)]);
    assert_eq!(layout(&project, "A1"), vec![row("a2", 0, 48)]);
    assert_eq!(layout(&project, "A2"), vec![row("music", 0, 192)]);
    assert!(!project.sequences[0].track("A2").unwrap().sync_locked);

    // Ripple trim: the edited tracks move even when they are not
    // sync-locked themselves; the bed stays.
    let project = run(
        &scene,
        vec![
            sync("V1", false),
            sync("A1", false),
            sync("A2", false),
            Op::Trim {
                clip: id("c1"),
                edge: Edge::Out,
                delta: f(-12),
                ripple: true,
            },
        ],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 36), row("c2", 36, 84)]
    );
    assert_eq!(
        layout(&project, "A1"),
        vec![row("a1", 0, 36), row("a2", 36, 84)]
    );
    assert_eq!(layout(&project, "A2"), vec![row("music", 0, 192)]);

    // Insert: only the target and the sync-locked tracks open.
    let project = run(
        &scene,
        vec![
            sync("A1", false),
            Op::Insert {
                sequence: id("s1"),
                at: f(48),
                duration: f(24),
                targets: vec![(id("V1"), new_media(480))],
            },
        ],
    );
    assert_eq!(
        layout(&project, "V1"),
        vec![row("c1", 0, 48), row("c3", 48, 72), row("c2", 72, 120)]
    );
    assert_eq!(
        layout(&project, "A1"),
        vec![row("a1", 0, 48), row("a2", 48, 96)]
    );
    assert_eq!(
        layout(&project, "A2"),
        vec![row("music", 0, 48), row("c4", 72, 216)]
    );

    // With every track sync-locked, ripple behaves as before.
    assert_eq!(
        rejects(
            &run(&scene, vec![sync("A2", true)]),
            vec![Op::RippleDelete {
                clips: vec![id("c1")]
            }]
        ),
        "ripple.blocked"
    );
}

#[test]
fn track_roles_take_known_spellings_and_clear() {
    let set = |role: Option<&str>| Op::SetTrackRole {
        sequence: id("s1"),
        track: id("A2"),
        change: TrackRoleChange {
            role: Some(role.map(str::to_owned)),
            sync_locked: None,
        },
    };
    let role = |project: &Project| project.sequences[0].track("A2").unwrap().role.clone();
    let music = run(&scene(), vec![set(Some(" Music "))]);
    assert_eq!(role(&music).as_deref(), Some("music"));
    assert!(music.sequences[0].track("A2").unwrap().sync_locked);
    let free = run(&music, vec![set(Some("Room tone"))]);
    assert_eq!(role(&free).as_deref(), Some("Room tone"));
    assert_eq!(role(&run(&free, vec![set(None)])), None);
    assert_eq!(role(&run(&free, vec![set(Some("  "))])), None);
    assert_eq!(rejects(&scene(), vec![set(Some("a\nb"))]), "track.role");
    let missing = Op::SetTrackRole {
        sequence: id("s1"),
        track: id("A9"),
        change: TrackRoleChange::default(),
    };
    assert_eq!(rejects(&scene(), vec![missing]), "ref.missing");
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
                speed: Ratio::parse("2").unwrap().into(),
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
        version: 1,
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

/// What `m1` would be read from another file: `frames` long at 24 fps,
/// with picture and sound as asked.
fn camera_take(frames: i64, video: bool, audio: bool) -> Media {
    Media {
        id: id("m1"),
        path: "media/take2.mov".into(),
        rate: Rate::FPS_24,
        drop_frame: false,
        start: Time::ZERO,
        duration: f(frames),
        video: video.then_some(VideoInfo {
            width: 1280,
            height: 720,
            alpha: false,
            ..Default::default()
        }),
        audio: audio.then_some(AudioInfo {
            channels: 2,
            sample_rate: 48_000,
        }),
        reel: None,
    }
}

/// The codes and entities of a rejection's diagnostics, sorted. Each
/// message names its entity.
fn rejection(project: &Project, ops: Vec<Op>) -> Vec<(&'static str, String)> {
    let rejected = apply(project, Catalog::builtin(), &ops).expect_err("expected a rejection");
    let mut found: Vec<_> = rejected
        .diagnostics
        .iter()
        .map(|d| {
            let entity = d.entity.as_ref().map(Id::to_string).unwrap_or_default();
            assert!(
                d.message.contains(&format!("`{entity}`")),
                "{} does not name {entity}",
                d.message
            );
            (d.code, entity)
        })
        .collect();
    found.sort();
    found
}

#[test]
fn replacing_media_keeps_its_id_and_every_clip_s_timing() {
    let scene = scene();
    let replacement = Media {
        id: id("m1"),
        path: "media/take2-0123abcd.mov".into(),
        rate: Rate::FPS_30,
        drop_frame: false,
        start: Rate::FPS_30.time_of(30 * 3600),
        duration: Rate::FPS_30.time_of(900),
        video: Some(VideoInfo {
            width: 3840,
            height: 2160,
            alpha: true,
            ..Default::default()
        }),
        audio: Some(AudioInfo {
            channels: 6,
            sample_rate: 96_000,
        }),
        reel: Some("B002".into()),
    };
    let (project, report) = apply(
        &scene,
        Catalog::builtin(),
        &[Op::ReplaceMedia(replacement.clone())],
    )
    .unwrap();
    assert_eq!(project.media("m1"), Some(&replacement));
    assert_eq!(project.media[1], scene.media[1]);
    assert_eq!(project.sequences, scene.sequences);
    assert_eq!(report, kerf_model::Report::default());
    assert_eq!(source_in(&project, "c2"), 240);
}

#[test]
fn replacing_media_refuses_a_file_its_clips_no_longer_fit() {
    let scene = scene();
    // c2 and a2 play 240–288 of m1.
    assert_eq!(
        rejection(&scene, vec![Op::ReplaceMedia(camera_take(200, true, true))]),
        vec![
            ("clip.source_range", "a2".to_owned()),
            ("clip.source_range", "c2".to_owned())
        ]
    );
    assert_eq!(
        rejection(
            &scene,
            vec![Op::ReplaceMedia(camera_take(2400, true, false))]
        ),
        vec![
            ("clip.domain", "a1".to_owned()),
            ("clip.domain", "a2".to_owned())
        ]
    );
    assert_eq!(
        rejection(
            &scene,
            vec![Op::ReplaceMedia(camera_take(2400, false, true))]
        ),
        vec![
            ("clip.domain", "c1".to_owned()),
            ("clip.domain", "c2".to_owned())
        ]
    );
    let mut unknown = camera_take(2400, true, true);
    unknown.id = id("m9");
    assert_eq!(
        rejects(&scene, vec![Op::ReplaceMedia(unknown)]),
        "ref.missing"
    );
    assert_eq!(
        rejects(&scene, vec![Op::ReplaceMedia(camera_take(0, true, true))]),
        "media.duration"
    );
}

#[test]
fn replacing_media_refuses_a_file_too_short_for_a_transition_s_handles() {
    // c1 plays 240–288 and dissolves into c2, which plays 24–72.
    let mut scene = scene();
    scene.sequences[0].tracks[0].clips[0] = media_clip("c1", "m1", 240, 0, 48, Some("L1"));
    scene.sequences[0].tracks[0].clips[1] = media_clip("c2", "m1", 24, 48, 48, Some("L2"));
    scene.sequences[0].tracks[1].clips[0] = media_clip("a1", "m1", 240, 0, 48, Some("L1"));
    scene.sequences[0].tracks[1].clips[1] = media_clip("a2", "m1", 24, 48, 48, Some("L2"));
    let dissolved = run(
        &scene,
        vec![Op::AddTransition {
            id: Some(id("t1")),
            from: id("c1"),
            to: id("c2"),
            fx: id("dissolve"),
            version: 1,
            duration: f(12),
            align: Align::Center,
            params: Params::new(),
        }],
    );
    // Every clip still fits 290 frames, but the dissolve needs 6 frames
    // after c1's last.
    let rejected = apply(
        &dissolved,
        Catalog::builtin(),
        &[Op::ReplaceMedia(camera_take(290, true, true))],
    )
    .unwrap_err();
    assert_eq!(rejected.diagnostics.len(), 1);
    let diagnostic = &rejected.diagnostics[0];
    assert_eq!(diagnostic.code, "transition.handles");
    assert!(
        diagnostic.message.contains("`c1`") && diagnostic.message.contains("`c2`"),
        "{}",
        diagnostic.message
    );
    run(
        &dissolved,
        vec![Op::ReplaceMedia(camera_take(294, true, true))],
    );
}
