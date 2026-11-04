use kerf_lang::{apply_script, parse_document, preset, print_document, PrintOptions};
use kerf_model::{Catalog, Content, Id, Project, Time};
use std::collections::BTreeMap;

const EDIT: &str = r#"kerf 1
project name="Original"
media m1 path="/private/original-footage.mov" rate=30 tc=01:00:00:00 dur=120s video=1920x1080 audio=2ch@48000 reel="PRIVATE_REEL"
sequence s1 name="Edit" rate=30 size=1080x1920 start=01:00:00:00
  track V1 video name="Picture"
    clip c1 media=m1 src=01:00:10:00 at=01:00:00:00 dur=2s link=L1 name="Opening"
      set scale={0f:1 30f:1.2:ease}
      fx color.basic@1 saturation=0.8 enabled=false
    clip c2 media=m1 src=01:00:30:00 at=01:00:02:00 dur=2s speed=2 link=L2 name="Detail"
    transition t1 from=c1 to=c2 fx=dissolve@1 dur=12f
  track A1 audio muted=true role=music sync=off
    clip a1 media=m1 src=01:00:10:00 at=01:00:00:00 dur=2s link=L1
      set gain=-3
    clip a2 media=m1 src=01:00:30:00 at=01:00:02:00 dur=2s speed=2 link=L2
    transition t2 from=a1 to=a2 fx=crossfade@1 dur=12f
  track V2 video locked=true role=titles
    clip title1 gen=title@1 at=01:00:00:00 dur=4s
      set text="Original title"
  track S1 subtitle
    clip cap1 caption="Original caption" at=01:00:01:00 dur=1s
  marker mk1 at=01:00:02:00 name="Beat"
"#;

fn parse(text: &str) -> Project {
    parse_document(text, Catalog::builtin()).unwrap().project
}

fn destination() -> Project {
    parse(
        r#"kerf 1
project name="New project" retired=p1,p2
media replacement path="new.mp4" rate=24 tc=10:00:00:00 dur=10s video=640x360 audio=2ch@48000
sequence target rate=25 size=1920x1080
  track Old video
    clip old media=replacement at=0f dur=1s
  marker oldmarker at=0f
sequence unrelated rate=30 size=320x180
  track Keep video
    clip keep gen=solid@1 at=0f dur=1s
      set color=#ff0000
"#,
    )
}

fn bindings() -> BTreeMap<String, preset::Binding> {
    ["slot1", "slot2"]
        .into_iter()
        .map(|slot| {
            (
                slot.into(),
                preset::Binding {
                    media: Id::new("replacement").unwrap(),
                    from: Time::ZERO,
                    beats: Vec::new(),
                },
            )
        })
        .collect()
}

fn exported() -> String {
    preset::export(
        &parse(EDIT),
        Catalog::builtin(),
        &Id::new("s1").unwrap(),
        "Product pattern",
    )
    .unwrap()
}

#[test]
fn exports_independent_cuts_but_keeps_linked_audio_in_the_same_slot() {
    let document = exported();
    assert!(!document.contains("original-footage"));
    assert!(!document.contains("PRIVATE_REEL"));
    let pattern = preset::parse(&document, Catalog::builtin())
        .unwrap()
        .pattern;
    assert_eq!(pattern.media.len(), 2);
    assert_eq!(pattern.name, "Product pattern");
    assert!((pattern.media[0].duration.seconds() - 2.2).abs() < 1e-9);
    assert!((pattern.media[1].duration.seconds() - 4.4).abs() < 1e-9);
    for media in &pattern.media {
        assert!(media.video.is_some() && media.audio.is_some());
        assert_eq!(media.start, Time::ZERO);
    }
    assert_eq!(
        pattern.clip("c1").unwrap().content,
        pattern.clip("a1").unwrap().content
    );
    assert_eq!(
        pattern.clip("c2").unwrap().content,
        pattern.clip("a2").unwrap().content
    );
    assert_eq!(
        pattern
            .clip("c2")
            .unwrap()
            .source_range()
            .unwrap()
            .0
            .seconds(),
        0.4
    );
    assert_eq!(exported(), document);
}

#[test]
fn applies_pattern_with_new_media_exact_times_effects_links_and_text() {
    let original = parse(EDIT);
    let destination = destination();
    let mut bindings = bindings();
    bindings.get_mut("slot2").unwrap().from = Time::from_seconds(1.0).unwrap();
    let texts = BTreeMap::from([
        ("title1".into(), "New title".into()),
        ("cap1".into(), "New caption".into()),
    ]);
    let script = preset::instantiate(
        &destination,
        Catalog::builtin(),
        &exported(),
        &Id::new("target").unwrap(),
        &bindings,
        &texts,
    )
    .unwrap();
    assert_eq!(
        script,
        preset::instantiate(
            &destination,
            Catalog::builtin(),
            &exported(),
            &Id::new("target").unwrap(),
            &bindings,
            &texts
        )
        .unwrap()
    );
    let applied = apply_script(&destination, Catalog::builtin(), &script)
        .unwrap()
        .project;
    let sequence = applied.sequence("target").unwrap();
    let source = &original.sequences[0];
    assert_eq!(sequence.rate, source.rate);
    assert_eq!(
        (
            sequence.width,
            sequence.height,
            sequence.start,
            sequence.duration()
        ),
        (source.width, source.height, source.start, source.duration())
    );
    for (track, source_track) in sequence.tracks.iter().zip(&source.tracks) {
        assert_eq!(
            (
                track.kind,
                track.enabled,
                track.locked,
                track.muted,
                &track.role,
                track.sync_locked
            ),
            (
                source_track.kind,
                source_track.enabled,
                source_track.locked,
                source_track.muted,
                &source_track.role,
                source_track.sync_locked
            )
        );
        for (clip, source_clip) in track.clips.iter().zip(&source_track.clips) {
            assert_eq!(
                (clip.start, clip.duration, clip.speed()),
                (source_clip.start, source_clip.duration, source_clip.speed())
            );
            assert_eq!(clip.props, source_clip.props);
            assert_eq!(clip.effects, source_clip.effects);
            assert!(clip.id != "p1" && clip.id != "p2");
        }
        assert_eq!(track.transitions.len(), source_track.transitions.len());
    }
    assert_eq!(
        sequence.tracks[0].clips[0].link,
        sequence.tracks[1].clips[0].link
    );
    assert_eq!(
        sequence.tracks[0].clips[1]
            .source_range()
            .unwrap()
            .0
            .seconds(),
        1.4
    );
    assert!(
        matches!(&sequence.tracks[3].clips[0].content, Content::Caption { text } if text == "New caption")
    );
    assert_eq!(
        applied.sequence("unrelated"),
        destination.sequence("unrelated")
    );
    assert_eq!(applied.media, destination.media);
    assert!(applied.retired.contains(&Id::new("old").unwrap()));
    let printed = print_document(&applied, Catalog::builtin(), PrintOptions::default());
    assert_eq!(parse(&printed), applied);
    assert!(printed.contains("New title"));
}

#[test]
fn refuses_missing_unknown_short_wrong_stream_and_negative_bindings() {
    let mut project = destination();
    let target = Id::new("target").unwrap();
    let document = exported();
    let attempt = |project: &Project, bindings: &BTreeMap<String, preset::Binding>| {
        preset::instantiate(
            project,
            Catalog::builtin(),
            &document,
            &target,
            bindings,
            &BTreeMap::new(),
        )
        .unwrap_err()
    };
    assert!(attempt(&project, &BTreeMap::new()).contains("missing binding"));
    let mut input = bindings();
    input.insert("unknown".into(), input["slot1"].clone());
    assert!(attempt(&project, &input).contains("unknown slot"));
    project.media[0].duration = Time::from_seconds(4.3).unwrap();
    assert!(attempt(&project, &bindings()).contains("including transition handles"));
    project.media[0].duration = Time::from_seconds(10.0).unwrap();
    project.media[0].audio = None;
    assert!(attempt(&project, &bindings()).contains("needs audio, which `replacement` lacks"));
    project = destination();
    input = bindings();
    input.get_mut("slot1").unwrap().from = Time::from_seconds(-1.0).unwrap();
    assert!(attempt(&project, &input).contains("needs"));
    project.sequences[0].tracks[0].locked = true;
    assert!(attempt(&project, &bindings()).contains("unlock"));
}

#[test]
fn validates_portability_and_text_override_keys() {
    assert!(preset::parse(
        &exported().replace("slot:slot1", "/private/missing.mov"),
        Catalog::builtin()
    )
    .is_err());
    assert!(preset::parse(
        &exported().replacen("kerf 1", "kerf 999", 1),
        Catalog::builtin()
    )
    .is_err());
    let texts = BTreeMap::from([("unknown".into(), "text".into())]);
    assert!(preset::instantiate(
        &destination(),
        Catalog::builtin(),
        &exported(),
        &Id::new("target").unwrap(),
        &bindings(),
        &texts
    )
    .unwrap_err()
    .contains("unknown text slot"));
}

#[test]
fn preserves_nested_sequences_and_reverse_transition_handles() {
    let original = parse(
        r#"kerf 1
project name="Nested"
media m path="private.mov" rate=30 dur=30s video=320x180
sequence main rate=30 size=640x360
  track V1 video
    clip nest seq=child at=0f dur=4s
sequence child rate=30 size=320x180
  track V1 video
    clip first media=m src=10s at=0f dur=2s speed=-1
    clip second media=m src=20s at=2s dur=2s speed=-1
    transition fade from=first to=second fx=dissolve@1 dur=12f
sequence unused rate=30 size=320x180
"#,
    );
    let document = preset::export(
        &original,
        Catalog::builtin(),
        &Id::new("main").unwrap(),
        "Nested",
    )
    .unwrap();
    let template = preset::parse(&document, Catalog::builtin())
        .unwrap()
        .pattern;
    assert_eq!(template.sequences.len(), 2);
    assert_eq!(template.media[0].duration.seconds(), 2.2);
    assert_eq!(template.media[1].duration.seconds(), 2.2);
    let destination = destination();
    let script = preset::instantiate(
        &destination,
        Catalog::builtin(),
        &document,
        &Id::new("target").unwrap(),
        &bindings(),
        &BTreeMap::new(),
    )
    .unwrap();
    let applied = apply_script(&destination, Catalog::builtin(), &script)
        .unwrap()
        .project;
    let nested = &applied.sequence("target").unwrap().tracks[0].clips[0];
    let Content::Sequence { sequence, .. } = &nested.content else {
        panic!("nested content");
    };
    assert_eq!(
        applied.sequence(sequence.as_str()).unwrap().tracks[0]
            .transitions
            .len(),
        1
    );
}

#[test]
fn exports_transitions_between_retimed_clips_at_every_rate() {
    // Handles that round to the nearest flick can fall one flick short of
    // what the validator requires, e.g. at 29.97 with 37% speed.
    for rate in ["23.976", "25", "29.97", "30", "59.94"] {
        for speed in ["0.37", "-0.37", "0.3333", "7/13"] {
            for frames in 1..=24 {
                for align in ["center", "start", "end"] {
                    let original = parse(&format!(
                        r#"kerf 1
project name="Retimed"
media m path="private.mov" rate={rate} dur=600s video=320x180
sequence s rate={rate} size=320x180
  track V1 video
    clip a media=m src=3000f at=0f dur=90f speed={speed}
    clip b media=m src=9000f at=90f dur=90f speed={speed}
    transition t from=a to=b fx=dissolve@1 dur={frames}f align={align}
"#
                    ));
                    preset::export(&original, Catalog::builtin(), &Id::new("s").unwrap(), "P")
                        .unwrap_or_else(|e| {
                            panic!("{rate}fps speed {speed}, {frames}f {align}: {e}")
                        });
                }
            }
        }
    }
}

#[test]
fn latest_start_is_the_last_whole_frame_that_fills_the_slot() {
    let pattern = preset::parse(&exported(), Catalog::builtin())
        .unwrap()
        .pattern;
    let slot = pattern.media("slot1").unwrap();
    let mut media = destination().media[0].clone();
    // 10s at 24 fps less 2.2s of slot leaves 7.8s: frame 187.2, so frame 187.
    assert_eq!(
        preset::latest_start(slot, &media),
        Some(media.rate.time_of(187))
    );
    media.duration = slot.duration;
    assert_eq!(preset::latest_start(slot, &media), Some(Time::ZERO));
    media.duration = Time::from_seconds(2.0).unwrap();
    assert_eq!(preset::latest_start(slot, &media), None);
    media.duration = Time::from_seconds(10.0).unwrap();
    media.audio = None;
    assert_eq!(preset::latest_start(slot, &media), None);
}

#[test]
fn slot_ids_do_not_collide_with_existing_clip_ids() {
    let original = parse(&EDIT.replace("title1", "slot1"));
    let document = preset::export(
        &original,
        Catalog::builtin(),
        &Id::new("s1").unwrap(),
        "Pattern",
    )
    .unwrap();
    preset::parse(&document, Catalog::builtin()).unwrap();
}

const MINE: &str = r#"kerf-look 1
# A look of this project's own.
look look.test.mine version=1 name="Mine" category="Test" on=clip
  doc "Nudges the picture at a hit."
  param hit time default=0f
set x={$hit:0 $hit+2f:0.1 $hit+4f:0}
"#;

/// A catalog with a project look beside the bundled ones.
fn with_mine() -> Catalog {
    let mut catalog = Catalog::builtin().clone();
    catalog
        .define_look(kerf_model::look::read(MINE, "mine.kerflook").unwrap())
        .unwrap();
    catalog
}

const BEATS: &str = r#"kerf 1
project name="Beats"
media m1 path="/private/song.mov" rate=30 dur=60s video=1920x1080 audio=2ch@48000
sequence s1 name="Beats" rate=30 size=1080x1920
  track V1 video
    clip c1 media=m1 src=0f at=0f dur=4s
      fx look.test.mine@1 hit=mk1
      fx look.shake@1 hit=mk2
  track A1 audio
    clip music media=m1 src=0f at=0f dur=4s
  marker mk1 at=30f
  marker mk2 at=90f
"#;

fn beats_preset() -> String {
    let catalog = with_mine();
    let original = parse_document(BEATS, &catalog).unwrap().project;
    preset::export(&original, &catalog, &Id::new("s1").unwrap(), "Beats").unwrap()
}

#[test]
fn a_preset_carries_the_looks_kerf_does_not_bundle() {
    let document = beats_preset();
    // The project's look travels, indented under its `look` line; the
    // bundled shake does not.
    assert!(
        document.contains("\nlook look.test.mine version=1")
            && document.contains("\n  set x={$hit:0"),
        "{document}"
    );
    assert!(!document.contains("look look.shake"), "{document}");
    // It reads where the look is not installed, and its time still names
    // the marker.
    assert!(Catalog::builtin().look("look.test.mine", 1).is_none());
    let read = preset::parse(&document, Catalog::builtin()).unwrap();
    assert_eq!(read.looks.len(), 1);
    assert!(read.catalog.look("look.test.mine", 1).is_some());
    assert!(document.contains("hit=mk1"), "{document}");
    // Where the look is installed already, it must be the same one.
    preset::parse(&document, &with_mine()).unwrap();
    let changed = document.replace("$hit+2f:0.1", "$hit+2f:0.5");
    let error = preset::parse(&changed, &with_mine()).unwrap_err();
    assert!(error.contains("is not the one in mine.kerflook"), "{error}");
    // A look the pattern does not use is not carried.
    let unused = format!(
        "{document}{}",
        MINE.replace("kerf-look 1\n", "")
            .replace("look.test.mine", "look.test.unused")
            .replace("\nset", "\n  set")
    );
    let error = preset::parse(&unused, Catalog::builtin()).unwrap_err();
    assert!(
        error.contains("`look.test.unused@1`, which it does not use"),
        "{error}"
    );
    // Errors in a carried look name the preset's own line.
    let broken = document.replace("param hit time", "param hit sometimes");
    let line = broken
        .lines()
        .position(|line| line.contains("param hit sometimes"))
        .unwrap()
        + 1;
    let error = preset::parse(&broken, Catalog::builtin()).unwrap_err();
    assert!(error.starts_with(&format!("preset:{line}:")), "{error}");
}

#[test]
fn markers_land_on_the_new_music_s_beats_and_looks_follow_them() {
    let document = beats_preset();
    let read = preset::parse(&document, Catalog::builtin()).unwrap();
    let slot = |audio: bool| {
        read.pattern
            .media
            .iter()
            .find(|m| m.audio.is_some() == audio && m.video.is_some() != audio)
            .unwrap()
            .id
            .to_string()
    };
    let destination = parse_document(
        r#"kerf 1
project name="New"
media pic path="pic.mov" rate=30 dur=10s video=1920x1080
media song path="song.wav" rate=30 dur=60s audio=2ch@48000
sequence target rate=30 size=1080x1920
"#,
        Catalog::builtin(),
    )
    .unwrap()
    .project;
    let second = |s: f64| Time::from_seconds(s).unwrap();
    let bindings = BTreeMap::from([
        (
            slot(false),
            preset::Binding {
                media: Id::new("pic").unwrap(),
                from: Time::ZERO,
                beats: Vec::new(),
            },
        ),
        (
            slot(true),
            preset::Binding {
                media: Id::new("song").unwrap(),
                from: second(1.0),
                // Media time; the clip plays it from 1s, so 0.9s is not heard.
                beats: [0.9, 2.2, 3.1, 4.4].map(second).to_vec(),
            },
        ),
    ]);
    let script = preset::instantiate(
        &destination,
        &read.catalog,
        &document,
        &Id::new("target").unwrap(),
        &bindings,
        &BTreeMap::new(),
    )
    .unwrap();
    let applied = apply_script(&destination, &read.catalog, &script)
        .unwrap()
        .project;
    let sequence = applied.sequence("target").unwrap();
    let rate = sequence.rate;
    let mut markers: Vec<_> = sequence.markers.iter().collect();
    markers.sort_by_key(|m| m.at);
    // 30f lands on 2.2s (1.2s in the sequence, 36f); 90f on 4.4s (102f),
    // nearer than 3.1s (63f).
    assert_eq!(
        markers
            .iter()
            .map(|m| rate.frames_round(m.at))
            .collect::<Vec<_>>(),
        [36, 102]
    );
    // The looks hit the markers they name, by their new ids.
    let expanded = kerf_model::look::expand(&applied, &read.catalog).unwrap();
    let clip = expanded
        .sequence("target")
        .unwrap()
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.props.contains_key("x"))
        .unwrap();
    let kerf_model::Param::Animated(keys) = &clip.props["x"] else {
        panic!("x animates")
    };
    let frames: Vec<_> = keys.iter().map(|k| rate.frames_round(k.at)).collect();
    assert!(
        frames.contains(&36) && frames.contains(&102) && frames[0] == 36,
        "{frames:?}"
    );
    let first = applied
        .sequence("target")
        .unwrap()
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .flat_map(|c| &c.effects)
        .find(|e| e.fx == "look.test.mine")
        .unwrap();
    assert_eq!(
        first.params["hit"],
        kerf_model::Param::Static(kerf_model::Value::Text(markers[0].id.to_string()))
    );
}

#[test]
fn the_presets_in_the_docs_read() {
    // A Windows checkout may end its lines with CRLF.
    let docs = include_str!("../../../docs/presets.md").replace("\r\n", "\n");
    let examples: Vec<&str> = docs
        .split("```text\n")
        .skip(1)
        .filter_map(|block| block.split("```").next())
        .filter(|block| block.starts_with("kerf 1"))
        .collect();
    assert_eq!(examples.len(), 2);
    for example in examples {
        preset::parse(example, Catalog::builtin()).unwrap_or_else(|e| panic!("{e}\n{example}"));
    }
}
