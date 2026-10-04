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
