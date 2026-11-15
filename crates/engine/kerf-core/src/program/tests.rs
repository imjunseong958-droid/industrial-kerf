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
fn sound_comes_from_audio_tracks_alone() {
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
    assert!(
        !argv.contains("/media/b.mov"),
        "picture clips carry no sound: {argv}"
    );
    let graph = &compiled.filter_complex;
    assert!(graph.contains("[0:a]"), "{graph}");
    assert_eq!(compiled.argv.iter().filter(|arg| *arg == "-map").count(), 1);
}

#[test]
fn the_graph_also_reads_clips_already_decoded() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=0f dur=48f\n\
         \x20   clip a2 media=m2 src=00:00:05:00 at=48f dur=24f\n",
    );
    let compiled = compile(&project);
    // What it reads, in the order of its inputs, as the arguments name them.
    let read: Vec<(&str, String, String)> = compiled
        .inputs
        .iter()
        .map(|input| (input.path.as_str(), secs(input.seek), secs(input.span)))
        .collect();
    assert_eq!(
        read,
        [
            ("/media/a.mov", "2".to_owned(), "2".to_owned()),
            ("/media/b.mov", "5".to_owned(), "1".to_owned())
        ]
    );
    assert_eq!(compiled.inputs[0].args(), ["-ss", "2", "-t", "2"]);
    // Each clip's chain starts with what makes its input stereo floats at the program's rate,
    // and the other graph is the same graph without that start.
    let (whole, decoded) = (compiled.filter_complex.replace('\n', ""), &compiled.decoded_filter);
    let mut put_back = decoded.clone();
    for (index, input) in compiled.inputs.iter().enumerate() {
        let start = format!("[{index}:a]{},", decoded_filters(input.span));
        assert!(whole.contains(&start), "{whole}");
        assert!(!decoded.contains(&decoded_filters(input.span)), "{decoded}");
        put_back = put_back.replace(&format!("[{index}:a]"), &start);
    }
    assert_eq!(put_back, whole);
    assert!(decoded.ends_with(&format!("[{}]", compiled.mixed)), "{decoded}");
    assert!(compiled.argv.ends_with(&["-map".to_owned(), format!("[{}]", compiled.mixed)]));
}

#[test]
fn crossfades_use_handles_and_fade_both_sides() {
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
    assert!(
        graph.contains("afade=t=out:st=1.75:d=0.5:curve=qsin"),
        "{graph}"
    );
    assert!(
        graph.contains("afade=t=in:st=0:d=0.5:curve=qsin"),
        "{graph}"
    );
    // The incoming clip starts 84000 samples in, while the outgoing one still sounds: each is a
    // stream of its own, as long as the program, and the two are summed.
    assert!(
        graph.contains("anullsrc=r=48000:cl=stereo,atrim=end_sample=84000[gap4]"),
        "{graph}"
    );
    assert!(
        graph.contains("[sound1]apad=whole_len=192000,atrim=end_sample=192000[piece3]"),
        "{graph}"
    );
    assert!(
        graph.contains("[sound2]apad=whole_len=108000,atrim=end_sample=108000[piece5]"),
        "{graph}"
    );
    assert!(
        graph.contains("[gap4][piece5]concat=n=2:v=0:a=1[lane6]"),
        "{graph}"
    );
    assert!(graph.contains("[bed0][piece3][lane6]amix=inputs=3"), "{graph}");
}

#[test]
fn clips_cut_one_after_another_are_one_stream() {
    // Three clips on a track with a gap before the last: laid end to end, each padded to the
    // sample the next starts on, so the mix sums one stream and not three.
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=0f dur=48f\n\
         \x20   clip a2 media=m2 src=00:00:02:00 at=48f dur=24f\n\
         \x20   clip a3 media=m1 src=00:00:06:00 at=96f dur=24f\n",
    );
    let graph = compile(&project).filter_complex;
    for piece in [
        "[sound1]apad=whole_len=96000,atrim=end_sample=96000[",
        "[sound2]apad=whole_len=96000,atrim=end_sample=96000[",
        "[sound3]apad=whole_len=48000,atrim=end_sample=48000[",
        "concat=n=3:v=0:a=1[",
        "amix=inputs=2:",
    ] {
        assert!(graph.contains(piece), "{piece} in {graph}");
    }
    assert!(!graph.contains("anullsrc") && !graph.contains("adelay"), "{graph}");
}

#[test]
fn a_speed_change_reads_the_right_source() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:02:00 at=0f dur=24f speed=-2\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:02:00 at=0f dur=24f speed=1/2\n",
    );
    let compiled = compile(&project);
    let argv = compiled.argv.join(" ");
    assert!(argv.contains("-ss 2 -t 0.5 -i /media/a.mov"), "{argv}");
    let graph = &compiled.filter_complex;
    assert!(graph.contains("atempo=0.5"), "{graph}");
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
fn the_sound_compiles_whatever_the_picture_is() {
    let project = project(
        "sequence s1 rate=24 size=1920x1080 audio=48000\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:02:00 at=00:00:00:00 dur=00:00:01:00\n\
         \x20     set blend=screen\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m2 src=00:00:02:00 at=00:00:00:00 dur=00:00:01:00\n",
    );
    // A blend is the compositor's to draw; the sound does not read it.
    let compiled = compile(&project);
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
