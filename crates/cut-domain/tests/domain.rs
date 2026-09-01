use cut_domain::{
    apply_history_command, apply_history_commands, create_history, empty_project, migrate_v1_to_v2,
    parse_command, parse_project, parse_v1, project_duration, redo, serialize_command,
    serialize_project, undo, Command, JsonValue, Revision, TrackType,
};
use serde_json::json;

#[test]
fn shared_protocol_golden_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/domain/parity.json")).unwrap();
    let expected = &fixture["expected"];

    let legacy = parse_v1(&fixture["v1"]).unwrap();
    let migrated = migrate_v1_to_v2(&legacy).unwrap();
    let starts: Vec<f64> = migrated.tracks[0]
        .clips
        .iter()
        .map(|clip| clip.timeline_start)
        .collect();
    assert_eq!(
        starts,
        expected["migrationStarts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect::<Vec<_>>()
    );

    let project = parse_project(&fixture["v2Aliases"]).unwrap();
    let canonical = serialize_project(&project).unwrap();
    assert_eq!(
        canonical["tracks"][0]["type"],
        expected["canonicalTrackType"]
    );
    assert_eq!(
        canonical["tracks"][0]["clips"][0]["sourceId"],
        expected["canonicalSourceId"]
    );
    assert_eq!(
        canonical["tracks"][0]["clips"][0]["timelineStart"].as_f64(),
        expected["canonicalTimelineStart"].as_f64()
    );
    assert!(canonical["tracks"][0]["clips"][0].get("src").is_none());
    assert_eq!(
        canonical["futureProject"],
        fixture["v2Aliases"]["futureProject"]
    );
    assert_eq!(
        canonical["sources"][0]["futureSource"],
        fixture["v2Aliases"]["sources"][0]["futureSource"]
    );
    assert_eq!(
        project_duration(&project),
        expected["duration"].as_f64().unwrap()
    );

    let commands: Vec<Command> = fixture["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| parse_command(command).unwrap())
        .collect();
    assert_eq!(
        serialize_command(&commands[0]).unwrap(),
        expected["serializedCommand"]
    );
    let initial = create_history(project.clone(), Revision::ZERO).unwrap();
    let applied = apply_history_commands(&initial, &commands, None).unwrap();
    assert_eq!(
        applied.revision.get(),
        expected["revisionAfterApply"].as_u64().unwrap()
    );
    let undone = undo(&applied, None).unwrap();
    assert_eq!(
        undone.revision.get(),
        expected["revisionAfterUndo"].as_u64().unwrap()
    );
    assert_eq!(undone.project, project);
    let redone = redo(&undone, None).unwrap();
    assert_eq!(
        redone.revision.get(),
        expected["revisionAfterRedo"].as_u64().unwrap()
    );
}

fn project_value() -> serde_json::Value {
    json!({
        "version": 2,
        "name": "Fixture",
        "fps": 30,
        "sources": [{"id":"src","path":"/tmp/a.mp4","duration":10,"futureSource":{"x":1}}],
        "tracks": [{"id":"video","kind":"video","clips":[{
            "id":"c1","src":"src","sourceIn":0,"sourceOut":4,"start":2,"speed":2,
            "futureClip":{"enabled":true}
        }]}],
        "overlays": [{"kind":"watermark","id":"ov","futureOverlay":true}],
        "gaps": [{"timelineStart":8,"duration":2}],
        "markers": [{"id":"m","start":1}],
        "futureProject": {"new": true}
    })
}

#[test]
fn v2_aliases_canonicalize_and_extensions_round_trip() {
    let project = parse_project(&project_value()).unwrap();
    assert_eq!(project.tracks[0].kind, TrackType::Video);
    assert_eq!(
        project.tracks[0].clips[0]
            .source_id
            .as_ref()
            .unwrap()
            .as_str(),
        "src"
    );
    assert_eq!(project.tracks[0].clips[0].timeline_start, 2.0);
    let serialized = serialize_project(&project).unwrap();
    assert_eq!(serialized["tracks"][0]["type"], "video");
    assert_eq!(serialized["tracks"][0]["clips"][0]["sourceId"], "src");
    assert_eq!(serialized["tracks"][0]["clips"][0]["timelineStart"], 2.0);
    assert_eq!(serialized["sources"][0]["futureSource"]["x"], 1);
    assert_eq!(
        serialized["tracks"][0]["clips"][0]["futureClip"]["enabled"],
        true
    );
    assert_eq!(serialized["futureProject"]["new"], true);
}

#[test]
fn conflicting_aliases_are_rejected() {
    let mut value = project_value();
    value["tracks"][0]["type"] = json!("audio");
    value["tracks"][0]["kind"] = json!("video");
    let error = parse_project(&value).unwrap_err();
    assert_eq!(error.code, "ALIAS_CONFLICT");
}

#[test]
fn migration_accumulates_effective_duration_per_track() {
    let legacy = json!({
        "version":1,"fps":30,"sources":[{"id":"s","path":"/tmp/a.mp4"}],
        "tracks":[{"id":"v0","clips":[{"id":"a","src":"s","in":0,"out":4,"speed":2},{"id":"b","src":"s","in":4,"out":6}]}],
        "overlays":[]
    });
    let legacy = parse_v1(&legacy).unwrap();
    let migrated = migrate_v1_to_v2(&legacy).unwrap();
    assert_eq!(migrated.tracks[0].kind, TrackType::Video);
    assert_eq!(
        migrated.tracks[0]
            .clips
            .iter()
            .map(|clip| clip.timeline_start)
            .collect::<Vec<_>>(),
        vec![0.0, 2.0]
    );
}

#[test]
fn command_alias_precedence_and_atomic_history() {
    let project = parse_project(&json!({
        "version":2,"fps":30,"sources":[],"tracks":[{"id":"titles","type":"title","clips":[]}]
    }))
    .unwrap();
    let command = parse_command(&json!({"type":"setMetadata","command":"remove_track","kind":"remove_track","op":"remove_track","patch":{"title":"Cut"}})).unwrap();
    assert!(matches!(command, Command::SetMetadata { .. }));
    let history = create_history(project, Revision::ZERO).unwrap();
    let next = apply_history_command(&history, &command, Some(Revision::ZERO)).unwrap();
    assert_eq!(next.revision.get(), 1);
    let bad = parse_command(
        &json!({"type":"add_track","track":{"id":"titles","type":"title","clips":[]}}),
    )
    .unwrap();
    assert!(apply_history_command(&next, &bad, None).is_err());
    assert_eq!(next.revision.get(), 1);
}

#[test]
fn batch_undo_redo_and_empty_identity() {
    let history = create_history(empty_project(), Revision::ZERO).unwrap();
    let command = parse_command(&json!({"type":"set_metadata","patch":{"x":true}})).unwrap();
    let next = apply_history_commands(&history, std::slice::from_ref(&command), None).unwrap();
    assert!(next.can_undo());
    let undone = undo(&next, Some(Revision::new(1))).unwrap();
    assert!(undone.can_redo());
    let redone = redo(&undone, Some(Revision::new(2))).unwrap();
    assert_eq!(
        redone.project.metadata.as_ref().unwrap().get("x"),
        Some(&JsonValue::Bool(true))
    );
    let no_op = undo(&history, Some(Revision::ZERO)).unwrap();
    assert_eq!(no_op, history);
}

#[test]
fn duration_uses_project_then_clip_duration_then_source_interval_and_gaps() {
    let project = parse_project(&json!({
        "version":2,"fps":30,"duration":1.5,"sources":[{"id":"s","path":"/tmp/a","duration":20}],
        "tracks":[{"id":"v","type":"video","clips":[
            {"id":"a","sourceId":"s","in":0,"out":4,"timelineStart":2,"duration":4},
            {"id":"b","sourceId":"s","in":1,"out":5,"timelineStart":8,"speed":2}
        ]}],"gaps":[{"start":12,"duration":3}]
    }))
    .unwrap();
    assert_eq!(project_duration(&project), 15.0);
}

#[test]
fn command_serialization_is_canonical() {
    let command = parse_command(&json!({"op":"setMetadata","patch":{"a":1}})).unwrap();
    assert_eq!(
        serialize_command(&command).unwrap(),
        json!({"type":"set_metadata","patch":{"a":1}})
    );
}
