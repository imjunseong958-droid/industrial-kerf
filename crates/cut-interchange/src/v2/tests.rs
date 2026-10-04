use super::*;
use cut_lang::{parse_document, print_document, PrintOptions};

const OVERLAP: &str = r#"{
  "version": 2,
  "name": "Imported",
  "fps": 24,
  "width": 1920,
  "height": 1080,
  "sources": [
    {"id": "src-1", "path": "/media/a.mov", "duration": 60, "width": 3840, "height": 2160, "hasAudio": true}
  ],
  "tracks": [
    {"id": "v1", "type": "video", "clips": [
      {"id": "clip-a", "sourceId": "src-1", "in": 0, "out": 5, "timelineStart": 0, "volume": 0.5,
       "opacity": 0.8, "crop": {"x": 0.1, "y": 0, "width": 0.8, "height": 1},
       "transform": {"x": 0.25, "scale": 1.5},
       "keyframes": [
         {"property": "opacity", "time": 0, "value": 0},
         {"property": "opacity", "time": 1, "value": 1}
       ],
       "transitions": [{"type": "dissolve", "duration": 1}]},
      {"id": "clip-b", "sourceId": "src-1", "in": 10, "out": 14, "timelineStart": 4}
    ]}
  ],
  "overlays": [{"id": "subs", "type": "captions", "cues": [{"start": 0.5, "end": 2, "text": "Hello"}]}],
  "gaps": [],
  "transitions": [],
  "effects": [],
  "markers": [{"id": "m1", "time": 2, "label": "beat", "color": "red"}]
}"#;

const EXPECTED: &str = r#"cut 1
project name="Imported"

media src-1 path="/media/a.mov" rate=24 dur=00:01:00:00 video=3840x2160 audio=2ch@48000

sequence s1 name="Imported" rate=24 size=1920x1080 audio=48000 start=00:00:00:00
  track v1 video
    clip clip-a media=src-1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:12 link=L1
      set x=0.25 scale=1.5 crop_l=0.1 crop_r=0.1 opacity={0f:0 24f:1}
    clip clip-b media=src-1 src=00:00:10:12 at=00:00:04:12 dur=00:00:03:12 link=L2
    transition t1 from=clip-a to=clip-b fx=dissolve dur=00:00:01:00
  track A1 audio
    clip clip-a_a media=src-1 src=00:00:00:00 at=00:00:00:00 dur=00:00:04:12 link=L1
      set gain=-6.02
    clip clip-b_a media=src-1 src=00:00:10:12 at=00:00:04:12 dur=00:00:03:12 link=L2
    transition t2 from=clip-a_a to=clip-b_a fx=crossfade dur=00:00:01:00
  track S1 subtitle
    clip cap1 caption="Hello" at=00:00:00:12 dur=00:00:01:12
  marker m1 at=00:00:02:00 name="beat" color=red
"#;

#[test]
fn overlaps_become_cuts_and_embedded_sound_gets_linked_clips() {
    let imported = import_json(OVERLAP).unwrap_or_else(|e| panic!("{e:#?}"));
    let text = print_document(
        &imported.project,
        Catalog::builtin(),
        PrintOptions::default(),
    );
    assert_eq!(text, EXPECTED);
    let reparsed = parse_document(&text, Catalog::builtin()).unwrap();
    assert_eq!(reparsed.project, imported.project);
    assert!(
        imported.warnings.iter().any(|w| w.contains("2ch@48000")),
        "{:?}",
        imported.warnings
    );
}

#[test]
fn imports_the_domain_parity_fixture_in_both_versions() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../../fixtures/domain/parity.json")).unwrap();
    for key in ["v1", "v2Aliases"] {
        let imported =
            import_json(&fixture[key].to_string()).unwrap_or_else(|e| panic!("{key}: {e:#?}"));
        let sequence = &imported.project.sequences[0];
        assert!(sequence.tracks.iter().any(|t| !t.clips.is_empty()), "{key}");
        let text = print_document(
            &imported.project,
            Catalog::builtin(),
            PrintOptions::default(),
        );
        assert_eq!(
            parse_document(&text, Catalog::builtin()).unwrap().project,
            imported.project,
            "{key}"
        );
    }
}

#[test]
fn overlaps_without_transitions_are_trimmed_and_reported() {
    let json = OVERLAP.replace(
        r#""transitions": [{"type": "dissolve", "duration": 1}]"#,
        r#""name": "a""#,
    );
    let imported = import_json(&json).unwrap();
    let video = &imported.project.sequences[0].tracks[0];
    assert!(video.transitions.is_empty());
    assert_eq!(video.clips[0].end(), video.clips[1].start);
    assert!(imported
        .warnings
        .iter()
        .any(|w| w.contains("without a transition")));
}

#[test]
fn explicit_audio_tracks_suppress_embedded_sound() {
    let mut value: serde_json::Value = serde_json::from_str(OVERLAP).unwrap();
    value["tracks"].as_array_mut().unwrap().push(serde_json::json!({
        "id": "a1", "type": "audio",
        "clips": [{"id": "music", "sourceId": "src-1", "in": 20, "out": 30, "timelineStart": 0, "volume": 1}]
    }));
    let imported = import_json(&value.to_string()).unwrap_or_else(|e| panic!("{e:#?}"));
    let tracks: Vec<&str> = imported.project.sequences[0]
        .tracks
        .iter()
        .map(|t| t.id.as_str())
        .collect();
    assert_eq!(tracks, ["v1", "a1", "S1"]);
    assert!(imported.project.clip("clip-a").unwrap().link.is_none());
}

#[test]
fn unusable_frame_rates_fail() {
    let json = OVERLAP.replace(r#""fps": 24"#, r#""fps": 37"#);
    let errors = import_json(&json).unwrap_err();
    assert!(errors[0].contains("frame rate"), "{errors:?}");
}

const KERNEL: &str = r#"cut 1
project name="Short Film" retired=c9,mk7

media m1 path="footage/A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000 reel="A001"
media m2 path="music.wav" rate=23.976 dur=00:03:00:00 audio=2ch@48000

sequence s1 name="Main Edit" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set x=0.1 scale={0f:1 48f:1.1:ease} opacity=0.9
      fx color.basic exposure=0.3 saturation=1.2
    clip c2 media=m1 src=14:23:02:00 at=01:00:06:12 dur=00:00:03:00 speed=-1.5
    transition t1 from=c1 to=c2 fx=dip dur=00:00:00:12 align=start color=#ffffff
  track V2 video name="Titles"
    clip c3 gen=title at=01:00:01:00 dur=00:00:02:00
      set y=0.3 text="Day One" size=0.1
  track S1 subtitle
    clip cap1 caption="Where are we?" at=01:00:01:00 dur=00:00:02:00
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set gain=-3.3333
  track A2 audio locked=true
    clip a2 media=m2 src=00:00:10:00 at=01:00:00:00 dur=00:00:20:00
      fx audio.eq3 low=-6
  marker mk1 at=01:00:05:00 name="pickup needed" color=red note="ADR"

sequence s2 name="Alt" rate=25 size=1280x720 audio=48000 start=00:00:00:00
  track A1 audio
    clip alt1 media=m2 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00
"#;

fn kernel() -> Project {
    parse_document(KERNEL, Catalog::builtin())
        .unwrap_or_else(|e| panic!("{e:#?}"))
        .project
}

/// Every clip's track, start, duration, and source in, by id.
fn timing(project: &Project) -> Vec<(String, String, Time, Time, Option<Time>)> {
    let mut out: Vec<_> = project.sequences[0]
        .tracks
        .iter()
        .flat_map(|track| {
            track.clips.iter().map(|clip| {
                (
                    track.id.to_string(),
                    clip.id.to_string(),
                    clip.start,
                    clip.duration,
                    clip.source_range().map(|range| range.0),
                )
            })
        })
        .collect();
    out.sort();
    out
}

#[test]
fn import_of_an_export_preserves_timing() {
    let project = kernel();
    let exported = export(&project).unwrap();
    let imported = import_v2(&exported).unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(timing(&imported.project), timing(&project));
    let transitions = &imported.project.sequences[0].tracks[0].transitions;
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].align, Align::Start);
    assert_eq!(transitions[0].fx, "dip");
    assert_eq!(
        imported.project.sequences[0].rate,
        project.sequences[0].rate
    );
}

#[test]
fn hinted_import_of_an_export_is_exact() {
    let project = kernel();
    let exported = export(&project).unwrap();
    let imported = import_v2_with(&exported, Some(&project)).unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(imported.warnings, Vec::<String>::new());
    assert_eq!(imported.project, project);
}

#[test]
fn hinted_import_takes_v2_edits_and_keeps_everything_else() {
    let project = kernel();
    let exported = export(&project).unwrap();
    let command = v2::parse_command(&serde_json::json!({
        "type": "batch",
        "commands": [
            {"type": "update_clip", "clipId": "c1", "patch": {"opacity": 0.5}},
            {"type": "remove_clip", "clipId": "c3"},
        ]
    }))
    .unwrap();
    let edited = v2::apply_command(&exported, &command).unwrap();
    let imported = import_v2_with(&edited, Some(&project)).unwrap_or_else(|e| panic!("{e:#?}"));
    assert!(imported.dropped.is_empty(), "{:?}", imported.dropped);

    let mut expected = project.clone();
    let c1 = expected.locate("c1").unwrap();
    expected
        .clip_at_mut(c1)
        .props
        .insert("opacity".into(), Param::Static(Value::Number(0.5)));
    expected.sequences[0].tracks[1].clips.clear();
    expected.retired.insert(Id::new("c3").unwrap());
    assert_eq!(imported.project, expected);
}

#[test]
fn hinted_import_reports_what_the_model_cannot_hold() {
    let project = kernel();
    let exported = export(&project).unwrap();
    let command = v2::parse_command(&serde_json::json!({
        "type": "set_metadata", "patch": {"reviewed": true}
    }))
    .unwrap();
    let edited = v2::apply_command(&exported, &command).unwrap();
    let imported = import_v2_with(&edited, Some(&project)).unwrap();
    assert_eq!(imported.project, project);
    assert_eq!(imported.dropped, ["project metadata was not imported"]);
}

#[test]
fn clips_v2_cannot_show_survive_a_v2_edit() {
    let text = KERNEL.replace(
        "      set y=0.3 text=\"Day One\" size=0.1\n",
        "      set y=0.3 text=\"Day One\" size=0.1\n    clip bg gen=solid at=01:00:03:00 dur=00:00:01:00\n    transition t5 from=c3 to=bg fx=dissolve dur=00:00:00:06\n",
    );
    let project = parse_document(&text, Catalog::builtin())
        .unwrap_or_else(|e| panic!("{e:#?}"))
        .project;
    let exported = export(&project).unwrap();
    let ids: Vec<&str> = exported.tracks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["V1", "V2", "V2/titles", "A1", "A2"]);
    assert_eq!(
        import_v2_with(&exported, Some(&project)).unwrap().project,
        project
    );
    let command = v2::parse_command(&serde_json::json!({
        "type": "update_clip", "clipId": "c3", "patch": {"opacity": 0.5}
    }))
    .unwrap();
    let edited = v2::apply_command(&exported, &command).unwrap();
    let imported = import_v2_with(&edited, Some(&project)).unwrap();
    let track = &imported.project.sequences[0].tracks[1];
    let clips: Vec<&str> = track.clips.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(clips, ["c3", "bg"]);
    assert_eq!(track.transitions.len(), 1);
}
