use super::*;
use kerf_lang::parse_document;

fn project(body: &str) -> Project {
    let text = format!(
        "kerf 1\nproject name=\"t\"\n\
         media m1 path=\"/media/a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\
         media m2 path=\"/media/b.mov\" rate=24 dur=00:01:00:00 video=1280x720 audio=2ch@48000\n\
         {body}"
    );
    parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
}

fn compile(project: &Project) -> CompiledProgram {
    compile_program(project, Catalog::builtin(), &ProgramOptions::default()).expect("compiles")
}

#[test]
fn seconds_and_samples_are_exact() {
    let ntsc = Rate::FPS_23_976;
    assert_eq!(secs(ntsc.time_of(1)), "0.041708333");
    assert_eq!(secs(ntsc.time_of(24_000)), "1001");
    assert_eq!(secs(Time::ZERO), "0");
    assert_eq!(secs(-Rate::FPS_24.time_of(12)), "-0.5");
    assert_eq!(samples(ntsc.time_of(1)), 2002);
    assert_eq!(samples(Rate::FPS_24.time_of(24)), 48_000);
}

#[test]
fn plain_cuts_concatenate_and_sound_comes_from_audio_tracks() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:02:00 at=00:00:00:00 dur=00:00:01:00\n\
         \x20   clip c2 media=m2 src=00:00:10:00 at=00:00:01:12 dur=00:00:00:12\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=00:00:00:00 dur=00:00:01:00\n",
    );
    let compiled = compile(&project);
    assert_eq!(compiled.duration, Rate::FPS_24.time_of(48));
    assert_eq!(compiled.rate_arg(), "24/1");
    let argv = compiled.argv.join(" ");
    assert!(argv.contains("-ss 2 -t 1 -i /media/a.mov"), "{argv}");
    assert!(argv.contains("-ss 10 -t 0.5 -i /media/b.mov"), "{argv}");
    let graph = &compiled.filter_complex;
    assert!(graph.contains("concat=n=3:v=1:a=0"), "{graph}");
    assert!(graph.contains("[0:a]"), "{graph}");
    assert!(
        !graph.contains("[1:a]"),
        "picture clips carry no sound: {graph}"
    );
    assert!(graph.contains("amix=inputs=2"), "{graph}");
}

#[test]
fn layered_clips_overlay_with_exact_ntsc_times() {
    let project = project(
        "sequence s1 rate=23.976 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=0f at=0f dur=48f\n\
         \x20 track V2 video\n\
         \x20   clip c2 media=m2 src=0f at=1f dur=24f\n\
         \x20     set opacity=0.5 x=0.25 scale={0f:1:ease 24f:2}\n",
    );
    let graph = compile(&project).filter_complex;
    assert!(graph.contains("r=24000/1001"), "{graph}");
    assert!(graph.contains("colorchannelmixer=aa=0.5"), "{graph}");
    assert!(
        graph.contains("enable='between(t,0.041708333,1.042708333)'"),
        "{graph}"
    );
    assert!(graph.contains("main_w*(0.25)"), "{graph}");
    assert!(graph.contains("eval=frame"), "{graph}");
    assert!(
        graph.contains("*(3-2*"),
        "ease keyframes keep their curve: {graph}"
    );
}

#[test]
fn transitions_use_handles_and_reveal_the_incoming_clip() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:02:00 at=0f dur=48f\n\
         \x20   clip c2 media=m2 src=00:00:02:00 at=48f dur=48f\n\
         \x20   transition t1 from=c1 to=c2 fx=dissolve dur=12f\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=0f dur=48f\n\
         \x20   clip a2 media=m2 src=00:00:02:00 at=48f dur=48f\n\
         \x20   transition t2 from=a1 to=a2 fx=crossfade dur=12f\n",
    );
    let compiled = compile(&project);
    let argv = compiled.argv.join(" ");
    // The outgoing clip plays 6 frames past the cut, the incoming one 6
    // frames before it.
    assert!(argv.contains("-ss 2 -t 2.25 -i /media/a.mov"), "{argv}");
    assert!(argv.contains("-ss 1.75 -t 2.25 -i /media/b.mov"), "{argv}");
    let graph = &compiled.filter_complex;
    assert!(graph.contains("fade=t=in:st=0:d=0.5:alpha=1"), "{graph}");
    assert!(
        graph.contains("afade=t=out:st=1.75:d=0.5:curve=qsin"),
        "{graph}"
    );
    assert!(
        graph.contains("afade=t=in:st=0:d=0.5:curve=qsin"),
        "{graph}"
    );
    assert!(
        graph.contains("adelay=delays=84000S:all=1,asetpts=N/SR/TB"),
        "{graph}"
    );
}

#[test]
fn a_range_keeps_a_transition_that_began_before_it() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:02:00 at=0f dur=48f\n\
         \x20   clip c2 media=m2 src=00:00:02:00 at=48f dur=48f\n\
         \x20   transition t1 from=c1 to=c2 fx=dissolve dur=12f\n",
    );
    let options = ProgramOptions {
        range: Some((Rate::FPS_24.time_of(48), Rate::FPS_24.time_of(72))),
        ..ProgramOptions::default()
    };
    let compiled = compile_program(&project, Catalog::builtin(), &options).unwrap();
    assert_eq!(compiled.duration, Rate::FPS_24.time_of(24));
    let graph = &compiled.filter_complex;
    // c2 is decoded from the transition's start and trimmed to the range.
    assert!(
        graph.contains("fade=t=in:st=0:d=0.5:alpha=1,trim=start=0.25:duration=1"),
        "{graph}"
    );
    assert!(graph.contains("trim=start=0:duration=0.25"), "{graph}");
}

#[test]
fn generators_captions_and_adjustment_layers_render() {
    let project = project(
        "sequence s1 rate=24 size=1080x1920 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip bg gen=solid at=0f dur=48f\n\
         \x20     set color=#336699\n\
         \x20 track V2 video\n\
         \x20   clip t gen=title at=0f dur=24f\n\
         \x20     set text=\"Hello, world\" y=0.3\n\
         \x20 track V3 video\n\
         \x20   clip adj gen=adjustment at=12f dur=24f\n\
         \x20     fx color.basic saturation=0\n\
         \x20 track S1 subtitle\n\
         \x20   clip cap caption=\"Hi\" at=0f dur=12f\n",
    );
    let compiled = compile(&project);
    assert_eq!((compiled.width, compiled.height), (1080, 1920));
    let graph = &compiled.filter_complex;
    assert!(graph.contains("color=c=0x336699"), "{graph}");
    assert!(graph.contains("drawtext=text=Hello\\,\\ world"), "{graph}");
    assert!(graph.contains("fontsize=153.6"), "{graph}");
    assert!(graph.contains("eq=saturation=0"), "{graph}");
    assert!(graph.contains("split"), "{graph}");
    assert!(graph.contains("text=Hi"), "{graph}");
}

#[test]
fn nested_sequences_composite_on_their_own_canvas() {
    let project = project(
        "sequence inner rate=24 size=1280x720 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip i1 media=m2 src=0f at=0f dur=48f\n\
         \x20 track A1 audio\n\
         \x20   clip i2 media=m2 src=0f at=0f dur=48f\n\
         sequence main rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip n1 seq=inner src=12f at=0f dur=24f\n\
         \x20 track A1 audio\n\
         \x20   clip n2 seq=inner src=12f at=0f dur=24f\n",
    );
    let options = ProgramOptions {
        sequence: Some("main".into()),
        ..ProgramOptions::default()
    };
    let compiled = compile_program(&project, Catalog::builtin(), &options).unwrap();
    let graph = &compiled.filter_complex;
    assert!(graph.contains("color=c=black@0:s=1280x720"), "{graph}");
    assert!(compiled
        .argv
        .join(" ")
        .contains("-ss 0.5 -t 1 -i /media/b.mov"));
    assert_eq!(graph.matches("amix").count(), 2, "{graph}");
}

#[test]
fn reverse_and_speed_change_read_the_right_source() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:02:00 at=0f dur=24f speed=-2\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=0f dur=24f speed=1/2\n",
    );
    let compiled = compile(&project);
    let argv = compiled.argv.join(" ");
    assert!(argv.contains("-ss 2 -t 2 -i /media/a.mov"), "{argv}");
    assert!(argv.contains("-ss 2 -t 0.5 -i /media/a.mov"), "{argv}");
    let graph = &compiled.filter_complex;
    assert!(graph.contains("reverse,setpts=PTS*1/2"), "{graph}");
    assert!(graph.contains("atempo=0.5"), "{graph}");
}

#[test]
fn features_without_a_lowering_are_named() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=0f at=0f dur=24f\n\
         \x20     set blend=multiply\n",
    );
    let error =
        compile_program(&project, Catalog::builtin(), &ProgramOptions::default()).unwrap_err();
    assert_eq!(error.entity(), Some("c1"));
    assert!(error.to_string().contains("blend `multiply`"), "{error}");
    let masked = self::project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=0f at=0f dur=24f\n\
         \x20     mask k1 rect=0.1,0.1,0.9,0.9\n",
    );
    let error =
        compile_program(&masked, Catalog::builtin(), &ProgramOptions::default()).unwrap_err();
    assert_eq!(error.entity(), Some("c1"));
    assert!(error.to_string().contains("masks"), "{error}");
}

#[test]
fn empty_sequences_are_refused() {
    let project = project("sequence s1 rate=24 size=1920x1080 audio=48000\n");
    assert!(matches!(
        compile_program(&project, Catalog::builtin(), &ProgramOptions::default()),
        Err(ProgramError::Invalid(_))
    ));
}

#[test]
fn the_sound_compiles_alone_for_a_picture_drawn_elsewhere() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:02:00 at=00:00:00:00 dur=00:00:01:00\n\
         \x20     set blend=screen\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m2 src=00:00:02:00 at=00:00:00:00 dur=00:00:01:00\n",
    );
    // The graph cannot blend the picture; the sound does not need it to.
    assert!(compile_program(&project, Catalog::builtin(), &ProgramOptions::default()).is_err());
    let compiled = compile_program(
        &project,
        Catalog::builtin(),
        &ProgramOptions {
            audio_only: true,
            ..ProgramOptions::default()
        },
    )
    .expect("compiles");
    let argv = compiled.argv.join(" ");
    assert!(argv.contains("-i /media/b.mov"), "{argv}");
    assert!(!argv.contains("/media/a.mov"), "{argv}");
    assert_eq!(compiled.argv.iter().filter(|arg| *arg == "-map").count(), 1);
    assert!(
        !compiled.filter_complex.contains("color="),
        "{}",
        compiled.filter_complex
    );
}

#[test]
fn a_range_after_a_crossfade_plays_the_incoming_clip_whole() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=0f dur=48f\n\
         \x20   clip a2 media=m2 src=00:00:02:00 at=48f dur=48f\n\
         \x20   transition t2 from=a1 to=a2 fx=crossfade dur=12f\n",
    );
    let f = |frames| Rate::FPS_24.time_of(frames);
    let options = ProgramOptions {
        range: Some((f(72), f(96))),
        audio_only: true,
        ..ProgramOptions::default()
    };
    // FFmpeg refuses a fade that starts before the stream does.
    let graph = compile_program(&project, Catalog::builtin(), &options)
        .unwrap()
        .filter_complex;
    assert!(!graph.contains("afade"), "{graph}");
    assert!(graph.contains("atrim=start=0:duration=1"), "{graph}");
}

fn sound_graph(project: &Project, range: Option<(Time, Time)>) -> String {
    compile_program(
        project,
        Catalog::builtin(),
        &ProgramOptions {
            range,
            audio_only: true,
            ..ProgramOptions::default()
        },
    )
    .expect("compiles")
    .filter_complex
}

#[test]
fn the_sound_comes_clip_by_clip_as_the_mix_chains_it() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=0f dur=48f\n\
         \x20   clip a2 media=m2 src=00:00:02:00 at=48f dur=48f\n\
         \x20   transition t2 from=a1 to=a2 fx=crossfade dur=12f\n\
         \x20 track A2 audio\n\
         \x20   clip b1 media=m2 src=0f at=24f dur=48f\n\
         \x20     set gain=-6\n\
         \x20     set pan=0.5\n\
         \x20 track A3 audio muted=true\n\
         \x20   clip x1 media=m2 src=0f at=0f dur=48f\n",
    );
    let f = |frames| Rate::FPS_24.time_of(frames);
    let catalog = Catalog::builtin();
    let sounds = program_sound(&project, catalog, None).unwrap();
    let spans: Vec<_> = sounds
        .iter()
        .map(|sound| (sound.clip.as_str(), sound.start, sound.end))
        .collect();
    // Each side of the crossfade plays through its handle.
    assert_eq!(
        spans,
        [
            ("a1", f(0), f(54)),
            ("b1", f(24), f(72)),
            ("a2", f(42), f(96))
        ]
    );
    let graph = sound_graph(&project, None);
    for sound in &sounds {
        let cut = sound.cut(catalog, Time::ZERO, f(96)).unwrap().unwrap();
        assert_eq!(cut.start, sound.start);
        assert!(graph.contains(&cut.filters), "{}\n{graph}", cut.filters);
    }
    // From inside the crossfade, the incoming clip decodes from the fade's
    // start, as a render of that range does.
    let cut = sounds[2].cut(catalog, f(45), f(96)).unwrap().unwrap();
    assert_eq!(cut.start, f(45));
    assert_eq!(cut.source, (f(48 - 6), f(48 + 48)));
    let graph = sound_graph(&project, Some((f(45), f(96))));
    assert!(graph.contains(&cut.filters), "{}\n{graph}", cut.filters);
    assert!(sounds[0].cut(catalog, f(54), f(96)).unwrap().is_none());
}

#[test]
fn nested_sound_plays_through_the_clips_holding_it() {
    let project = project(
        "sequence main rate=24 size=1920x1080 audio=48000\n\
         \x20 track A1 audio\n\
         \x20   clip n1 seq=inner src=12f at=24f dur=24f\n\
         \x20     set gain=-6\n\
         \x20   clip n2 seq=inner src=0f at=48f dur=12f speed=2\n\
         \x20 track V1 video\n\
         \x20   clip n3 seq=inner src=0f at=0f dur=48f\n\
         \x20 track A2 audio\n\
         \x20   clip n4 seq=inner src=0f at=72f dur=24f\n\
         \x20     fx audio.eq3 low=6\n\
         sequence inner rate=24 size=1920x1080 audio=48000\n\
         \x20 track A1 audio\n\
         \x20   clip i1 media=m1 src=0f at=0f dur=48f\n\
         \x20 track A2 audio\n\
         \x20   clip i2 media=m2 src=0f at=24f dur=24f\n\
         \x20 track A3 audio\n\
         \x20   clip i3 media=m2 src=0f at=0f dur=12f\n",
    );
    let f = |frames| Rate::FPS_24.time_of(frames);
    let catalog = Catalog::builtin();
    let sounds = program_sound(&project, catalog, None).unwrap();
    let spans: Vec<_> = sounds
        .iter()
        .map(|sound| (sound.clip.as_str(), sound.start, sound.end))
        .collect();
    // `n1` plays [12f, 36f) of `inner`; a picture track mixes no sound, and
    // `n2` changes the speed of its mix as a whole.
    assert_eq!(
        spans,
        [
            ("i1", f(24), f(48)),
            ("i2", f(36), f(48)),
            ("i1", f(72), f(96)),
            ("i3", f(72), f(84))
        ]
    );
    // The inner chain fills the window `n1` decodes, then `n1`'s plays.
    let cut = sounds[1].cut(catalog, f(36), f(48)).unwrap().unwrap();
    assert_eq!((cut.start, cut.source), (f(36), (f(0), f(12))));
    assert!(
        cut.filters.ends_with(
            ",apad,atrim=duration=0.5,asetpts=PTS-STARTPTS,\
             volume=-6dB,atrim=start=0:duration=0.5,asetpts=PTS-STARTPTS"
        ),
        "{}",
        cut.filters
    );
    // An effect on the holder rings on past the nested clip's end, inside
    // the holder.
    let cut = sounds[3].cut(catalog, f(72), f(96)).unwrap().unwrap();
    assert!(cut.filters.contains("bass=g=6:f=250"), "{}", cut.filters);
    assert!(
        cut.filters.contains("apad,atrim=duration=0.75,"),
        "{}",
        cut.filters
    );
}

#[test]
fn an_angle_plays_one_sound_track_of_a_sequence() {
    let catalog = Catalog::builtin();
    let document = |angle: &str| {
        format!("kerf 1\nproject name=\"t\"\nmedia m path=\"a.mov\" rate=24 dur=96f audio=2ch@48000\nmedia n path=\"b.mov\" rate=24 dur=96f audio=2ch@48000\nsequence s1 rate=24 size=64x36 audio=48000\n  track A1 audio\n    clip c1 seq=mc src=0f at=0f dur=24f{angle}\nsequence mc rate=24 size=64x36 audio=48000\n  track A1 audio\n    clip a media=m src=0f at=0f dur=48f\n  track A2 audio muted=true\n    clip b media=n src=0f at=0f dur=48f\n")
    };
    let heard = |angle: &str| {
        let project = kerf_lang::parse_document(&document(angle), catalog)
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let sounds = program_sound(&project, catalog, None).expect("sounds");
        let compiled = compile_program(
            &project,
            catalog,
            &ProgramOptions {
                audio_only: true,
                ..ProgramOptions::default()
            },
        )
        .expect("compiles");
        (
            sounds
                .iter()
                .map(|sound| sound.path.clone())
                .collect::<Vec<_>>(),
            compiled.argv.iter().any(|arg| arg == "b.mov"),
        )
    };
    assert_eq!(heard(""), (vec!["a.mov".to_owned()], false));
    assert_eq!(heard(" angle=A1"), (vec!["a.mov".to_owned()], false));
    // The camera with its track muted is heard when asked for.
    assert_eq!(heard(" angle=A2"), (vec!["b.mov".to_owned()], true));
}

#[test]
fn the_graph_refuses_a_picture_angle_by_name() {
    let catalog = Catalog::builtin();
    let project = kerf_lang::parse_document(
        "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 seq=mc src=0f at=0f dur=24f angle=V1\nsequence mc rate=24 size=64x36 audio=48000\n  track V1 video\n    clip a gen=solid@1 at=0f dur=48f\n",
        catalog,
    )
    .unwrap_or_else(|errors| panic!("{errors:?}"))
    .project;
    let refused = compile_program(&project, catalog, &ProgramOptions::default())
        .expect_err("the graph draws no angle");
    assert!(refused.to_string().contains("angle"), "{refused}");
}

#[test]
fn the_graph_refuses_what_only_the_compositor_draws_by_name() {
    let catalog = Catalog::builtin();
    for (clip, name) in [
        ("gen=shape@1 at=0f dur=24f", "shape"),
        ("gen=particles@1 at=0f dur=24f", "particles"),
        (
            "gen=title@1 at=0f dur=24f\n      set text=\"a\" reveal={0f:0 12f:1}",
            "reveal",
        ),
        (
            "gen=title@1 at=0f dur=24f\n      set text=\"a\" wght=700",
            "axes",
        ),
        (
            "gen=title@1 at=0f dur=24f\n      set text=\"a\" ital={0f:0 12f:1}",
            "axes",
        ),
    ] {
        let project = kerf_lang::parse_document(
            &format!("kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=64x36 audio=48000\n  track V1 video\n    clip c1 {clip}\n"),
            catalog,
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        let refused = compile_program(&project, catalog, &ProgramOptions::default())
            .expect_err("the graph draws none of these");
        assert!(refused.to_string().contains(name), "{refused}");
    }
}
