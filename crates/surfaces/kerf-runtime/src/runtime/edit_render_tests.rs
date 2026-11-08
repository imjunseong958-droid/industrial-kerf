//! Typed edits the editor sends (keyframes, markers, track roles and sync
//! lock) and the render queue's options and listing.

use super::*;
use crate::parse_document;
use serde_json::{json, Value};

/// A project folder with a 24 fps sequence: V1 holds c1 (0–2 s) and c2
/// (2–4 s), V2 a title at 3–4 s, and A1 sound linked to c1 and a later
/// clip at 3–4 s.
fn project(label: &str) -> (Runtime, PathBuf) {
    let directory = std::env::temp_dir().join(format!("kerf-e1-{label}-{}", unique_id()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(PROJECT_FILE),
        "kerf 1\nproject name=\"E1\"\n\n\
         media m1 path=\"a.mov\" rate=24 dur=00:01:00:00 video=1920x1080 audio=2ch@48000\n\n\
         sequence s1 name=\"Main\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n\
         \x20 track V1 video\n\
         \x20   clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00 link=L1\n\
         \x20   clip c2 media=m1 src=00:00:10:00 at=00:00:02:00 dur=00:00:02:00\n\
         \x20 track V2 video\n\
         \x20   clip t1 gen=title@1 at=00:00:03:00 dur=00:00:01:00\n\
         \x20     set text=\"Day One\"\n\
         \x20 track A1 audio\n\
         \x20   clip a1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:02:00 link=L1\n\
         \x20   clip a2 media=m1 src=00:00:20:00 at=00:00:03:00 dur=00:00:01:00\n",
    )
    .unwrap();
    (Runtime::open(&directory).unwrap(), directory)
}

/// Apply typed ops given as JSON at the current revision.
fn ops(runtime: &mut Runtime, ops: Value) -> Result<StateSnapshot, RuntimeError> {
    let ops: Vec<EditOp> = serde_json::from_value(ops).unwrap();
    let revision = runtime.state.revision;
    runtime.apply_ops_request("ui", &unique_id(), ops, revision)
}

fn clip(timeline: &Value, track: usize, clip: usize) -> &Value {
    &timeline["tracks"][track]["clips"][clip]
}

/// A timeline keyframe list as the `{"keys": [...]}` a typed op takes.
fn keys_of(keys: &Value) -> Value {
    let keys: Vec<Value> = keys
        .as_array()
        .unwrap()
        .iter()
        .map(|key| json!({"at": key["frame"], "value": key["value"], "interp": key["interp"]}))
        .collect();
    json!({ "keys": keys })
}

#[test]
fn keyframes_round_trip_through_typed_ops_and_the_timeline() {
    let (mut runtime, directory) = project("keys");
    // Keys out of order are put in order; interp defaults to linear.
    let state = ops(
        &mut runtime,
        json!([{"op": "set", "clip": "c1", "props": {"opacity": {"keys": [
            {"at": 24, "value": 1.0},
            {"at": 0, "value": 0.0, "interp": "ease"},
            {"at": 36, "value": 0.5, "interp": "hold"}
        ]}}}]),
    )
    .unwrap();
    let opacity = &clip(&state.timeline, 0, 0)["props"]["opacity"];
    assert_eq!(
        opacity,
        &json!([
            {"at": 0.0, "frame": 0, "value": 0.0, "interp": "ease"},
            {"at": 1.0, "frame": 24, "value": 1.0, "interp": "linear"},
            {"at": 1.5, "frame": 36, "value": 0.5, "interp": "hold"}
        ])
    );
    let project = parse_document(&state.document).unwrap();
    let kerf_model::Param::Animated(keys) = &project.clip("c1").unwrap().props["opacity"] else {
        panic!("opacity is animated: {}", state.document);
    };
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[2].interp, kerf_model::Interp::Hold);

    // What the timeline shows goes back as it came: on another clip, the
    // same keys.
    let state = ops(
        &mut runtime,
        json!([{"op": "set", "clip": "c2", "props": {"opacity": keys_of(opacity)}}]),
    )
    .unwrap();
    assert_eq!(&clip(&state.timeline, 0, 1)["props"]["opacity"], opacity);

    // Effects and masks take keys the same way.
    let state = ops(
        &mut runtime,
        json!([
            {"op": "fx-add", "clip": "c1", "fx": "light.edgeglow",
             "params": {"radius": {"keys": [{"at": 0, "value": 10}, {"at": 12, "value": 40}]}}},
            {"op": "fx-set", "clip": "c1", "index": 0,
             "params": {"intensity": {"keys": [{"at": 6, "value": 0.2}, {"at": 18, "value": 0.8, "interp": "ease-out"}]}}},
            {"op": "mask-add", "clip": "c1", "rect": [0.25, 0.25, 0.75, 0.75],
             "params": {"feather": {"keys": [{"at": 0, "value": 0}, {"at": 48, "value": 20}]}}}
        ]),
    )
    .unwrap();
    let c1 = clip(&state.timeline, 0, 0);
    assert_eq!(c1["effects"][0]["params"]["radius"][1]["frame"], 12);
    assert_eq!(c1["effects"][0]["params"]["radius"][1]["value"], 40.0);
    assert_eq!(
        c1["effects"][0]["params"]["intensity"][1]["interp"],
        "ease-out"
    );
    assert_eq!(c1["masks"][0]["params"]["feather"][1]["frame"], 48);
    assert_eq!(c1["masks"][0]["params"]["feather"][1]["at"], 2.0);

    // `null` takes the animation away again, back to the default.
    let state = ops(
        &mut runtime,
        json!([{"op": "set", "clip": "c2", "props": {"opacity": null}}]),
    )
    .unwrap();
    assert!(clip(&state.timeline, 0, 1).get("props").is_none());
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn a_clip_in_depth_reaches_the_document_as_the_inspector_sends_it() {
    let (mut runtime, directory) = project("in-depth");
    let state = ops(
        &mut runtime,
        json!([{"op": "set", "clip": "c1", "props": {
            "three_d": true, "z": 0.5, "rotate_x": 45.0, "rotate_y": -90.0, "depth": 0.1, "bevel": 0.05
        }}]),
    )
    .unwrap();
    let props = &clip(&state.timeline, 0, 0)["props"];
    assert_eq!(props["three_d"], json!(true));
    assert_eq!(props["z"], json!(0.5));
    assert_eq!(props["bevel"], json!(0.05));
    let project = parse_document(&state.document).unwrap();
    let parsed = &project.clip("c1").unwrap().props;
    assert_eq!(
        parsed["three_d"],
        kerf_model::Param::Static(kerf_model::Value::Bool(true)),
        "{}",
        state.document
    );
    for name in ["z", "rotate_x", "rotate_y", "depth", "bevel"] {
        assert!(parsed.contains_key(name), "{name}: {}", state.document);
    }
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn keyframes_that_cannot_play_are_refused_with_the_reason() {
    let (mut runtime, directory) = project("keys-refused");
    let before = runtime.document(false);
    let refused = |runtime: &mut Runtime, op: Value| {
        let error = ops(runtime, json!([op])).unwrap_err();
        (error.code, error.message)
    };
    // Clip gain is followed by playback and renders; pan and sound effects
    // are not.
    let (code, message) = refused(
        &mut runtime,
        json!({"op": "set", "clip": "a1", "props": {"pan": {"keys": [{"at": 0, "value": -1}, {"at": 24, "value": 1}]}}}),
    );
    assert_eq!(code, "INVALID_EDIT");
    assert!(
        message.contains("`pan` cannot be animated on sound"),
        "{message}"
    );
    let (_, message) = refused(
        &mut runtime,
        json!({"op": "fx-add", "clip": "a1", "fx": "audio.eq",
               "params": {"presence_db": {"keys": [{"at": 0, "value": 0}, {"at": 24, "value": 6}]}}}),
    );
    assert!(
        message.contains("`audio.eq` presence_db cannot be animated"),
        "{message}"
    );
    // A parameter the catalog does not animate.
    let (_, message) = refused(
        &mut runtime,
        json!({"op": "set", "clip": "c1", "props": {"blend": {"keys": [{"at": 0, "value": "normal"}]}}}),
    );
    assert!(message.contains("`blend` cannot be animated"), "{message}");
    // Keys that are not whole frames, an unknown curve, none at all, or a
    // value of the wrong type.
    for keys in [
        json!([{"at": 1.5, "value": 0.5}]),
        json!([{"at": 0, "value": 0.5, "interp": "bounce"}]),
        json!([]),
        json!([{"at": 0, "value": "half"}]),
        json!([{"value": 0.5}]),
    ] {
        let (code, message) = refused(
            &mut runtime,
            json!({"op": "set", "clip": "c1", "props": {"opacity": {"keys": keys}}}),
        );
        assert_eq!(code, "INVALID_EDIT", "{keys}");
        assert!(message.contains("`opacity` keyframes are"), "{message}");
    }
    // Two keys at one frame are refused when the edit is checked.
    let error = ops(
        &mut runtime,
        json!([{"op": "set", "clip": "c1", "props": {"opacity": {"keys": [
            {"at": 6, "value": 0.0}, {"at": 6, "value": 1.0}
        ]}}}]),
    )
    .unwrap_err();
    assert!(error.message.contains("strictly increasing"), "{error:?}");
    assert_eq!(runtime.document(false), before);

    // Keyframed gain is what sound takes: the rubber band.
    let state = ops(
        &mut runtime,
        json!([{"op": "set", "clip": "a1", "props": {"gain": {"keys": [
            {"at": 0, "value": -6}, {"at": 24, "value": 0, "interp": "ease"}
        ]}}}]),
    )
    .unwrap();
    assert_eq!(clip(&state.timeline, 2, 0)["props"]["gain"][1]["frame"], 24);
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn set_marker_moves_renames_and_clears_a_marker() {
    let (mut runtime, directory) = project("markers");
    ops(
        &mut runtime,
        json!([
            {"op": "add-marker", "id": "beat", "at": 12, "name": "beat", "color": "green"},
            {"op": "add-marker", "id": "out", "at": 60, "name": "out"}
        ]),
    )
    .unwrap();
    let state = ops(
        &mut runtime,
        json!([{"op": "set-marker", "marker": "beat", "at": 72, "frames": 12,
                "name": "Drop", "color": "red", "note": "cut on the hit"}]),
    )
    .unwrap();
    let markers = state.timeline["markers"].as_array().unwrap();
    // Markers stay in time order, so the moved one is now second.
    assert_eq!(markers[0]["id"], "out");
    assert_eq!(
        markers[1],
        json!({"id": "beat", "time": 3.0, "frame": 72, "duration": 0.5,
               "label": "Drop", "color": "red", "note": "cut on the hit"})
    );
    assert_eq!(
        runtime.state.past.last().unwrap().script.lines().count(),
        1,
        "one op, one line"
    );
    // Blank text clears; what is absent stays.
    let state = ops(
        &mut runtime,
        json!([{"op": "set-marker", "marker": "beat", "name": " ", "color": ""}]),
    )
    .unwrap();
    let beat = &state.timeline["markers"][1];
    assert!(
        beat.get("label").is_none() && beat.get("color").is_none(),
        "{beat}"
    );
    assert_eq!(
        (beat["frame"].as_i64(), beat["note"].as_str()),
        (Some(72), Some("cut on the hit"))
    );

    for (op, wanted) in [
        (
            json!({"op": "set-marker", "marker": "gone", "at": 1}),
            "no marker `gone`",
        ),
        (
            json!({"op": "set-marker", "marker": "beat", "color": "mauve"}),
            "`mauve` is not a marker color",
        ),
    ] {
        let error = ops(&mut runtime, json!([op])).unwrap_err();
        assert_eq!(error.code, "INVALID_EDIT");
        assert!(error.message.contains(wanted), "{error:?}");
    }
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn a_title_inserted_from_the_editor_is_kept_in_project_kerf_and_renders() {
    let (mut runtime, directory) = project("generator");
    // As the editor sends it: a title at the playhead on V2.
    let state = ops(
        &mut runtime,
        json!([{"op": "insert", "generator": {"fx": "title", "params": {"text": "Day Two"}},
                "at": 24, "frames": 12, "tracks": ["V2"]}]),
    )
    .unwrap();
    let v2 = &state.timeline["tracks"][1]["clips"];
    let title = v2
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["name"] == "Day Two")
        .unwrap_or_else(|| panic!("the new title on V2: {v2}"));
    assert_eq!(
        (title["kind"].as_str(), title["fx"].as_str()),
        (Some("generator"), Some("title"))
    );
    assert_eq!(
        (
            title["startFrame"].as_i64(),
            title["durationFrames"].as_i64()
        ),
        (Some(24), Some(12))
    );
    // Insert pushed the later title along by the new one's length.
    assert!(v2
        .as_array()
        .unwrap()
        .iter()
        .any(|clip| clip["name"] == "Day One" && clip["startFrame"].as_i64() == Some(84)));
    let id = title["id"].as_str().unwrap().to_owned();
    drop(runtime);

    // Reopened from the folder, project.kerf has it as the language writes it.
    let written = fs::read_to_string(directory.join(PROJECT_FILE)).unwrap();
    assert!(
        written.contains(&format!(
            "clip {id} gen=title@1 at=00:00:01:00 dur=00:00:00:12"
        )),
        "{written}"
    );
    assert!(written.contains("set text=\"Day Two\""), "{written}");
    let runtime = Runtime::open(&directory).unwrap();
    let project = parse_document(&runtime.document(false)).unwrap();
    let kerf_model::Content::Generator { fx, params, .. } = &project.clip(&id).unwrap().content
    else {
        panic!("{id} is a generator");
    };
    assert_eq!(fx.as_str(), "title");
    assert_eq!(
        params["text"],
        kerf_model::Param::Static(kerf_model::Value::Text("Day Two".into()))
    );
    // It draws as a render draws it: the title shows while it plays.
    let rate = kerf_model::Rate::FPS_24;
    let frame = kerf_engine::still::draw_frame(&project, None, rate.time_of(30), 0.25).unwrap();
    let lit = frame
        .rgba
        .as_chunks::<4>().0.iter()
        .filter(|pixel| pixel[..3].iter().any(|channel| *channel > 32))
        .count();
    assert!(lit > 0, "the title draws on the frame");
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn the_frame_rate_changes_through_set_sequence_only_while_the_sequence_is_empty() {
    let (mut runtime, directory) = project("rate");
    let error = ops(
        &mut runtime,
        json!([{"op": "set-sequence", "rate": {"num": 30, "den": 1}}]),
    )
    .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            "INVALID_EDIT",
            "Frame rate can change only while the sequence is empty"
        )
    );
    assert_eq!(runtime.snapshot().timeline["fps"], 24.0);
    // Emptied, it takes the rate with the name and size in one edit.
    let clips = ["c1", "c2", "t1", "a1", "a2"];
    ops(&mut runtime, json!([{"op": "delete", "clips": clips}])).unwrap();
    let state = ops(
        &mut runtime,
        json!([{"op": "set-sequence", "rate": {"num": 30, "den": 1}, "name": "Vertical",
                "width": 1080, "height": 1920}]),
    )
    .unwrap();
    assert_eq!(state.timeline["fps"], 30.0);
    assert_eq!(state.timeline["rate"], "30");
    assert_eq!(
        (
            state.timeline["width"].as_u64(),
            state.timeline["height"].as_u64()
        ),
        (Some(1080), Some(1920))
    );
    assert!(
        state
            .document
            .contains("sequence s1 name=\"Vertical\" rate=30 size=1080x1920"),
        "{}",
        state.document
    );
    // Undo puts the rate back.
    let revision = runtime.state.revision;
    let undone = runtime.undo_request("ui", &unique_id(), revision).unwrap();
    assert_eq!(undone.timeline["fps"], 24.0);
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn set_track_gives_roles_and_sync_lock() {
    let (mut runtime, directory) = project("roles");
    let state = runtime.snapshot();
    let tracks = state.timeline["tracks"].as_array().unwrap();
    // Unassigned tracks show their kind's role and are sync-locked.
    assert_eq!(
        (
            tracks[0]["role"].as_str(),
            tracks[0]["roleAssigned"].as_bool()
        ),
        (Some("video"), Some(false))
    );
    assert_eq!(tracks[2]["role"], "dialogue");
    assert!(tracks.iter().all(|track| track["syncLocked"] == true));

    let state = ops(
        &mut runtime,
        json!([
            {"op": "set-track", "track": "V2", "role": "Titles", "syncLocked": false},
            {"op": "set-track", "track": "A1", "role": "Music & FX", "locked": true}
        ]),
    )
    .unwrap();
    let tracks = state.timeline["tracks"].as_array().unwrap();
    assert_eq!(
        (
            tracks[1]["role"].as_str(),
            tracks[1]["roleAssigned"].as_bool(),
            tracks[1]["syncLocked"].as_bool()
        ),
        (Some("titles"), Some(true), Some(false))
    );
    assert_eq!(
        (tracks[2]["role"].as_str(), tracks[2]["locked"].as_bool()),
        (Some("Music & FX"), Some(true))
    );
    let document = runtime.document(false);
    assert!(
        document.contains("track V2 video role=titles sync=off"),
        "{document}"
    );
    assert!(document.contains("role=\"Music & FX\""), "{document}");

    // An empty role gives back the kind's; sync lock goes back on.
    let state = ops(
        &mut runtime,
        json!([{"op": "set-track", "track": "V2", "role": "", "syncLocked": true}]),
    )
    .unwrap();
    let v2 = &state.timeline["tracks"][1];
    assert_eq!(
        (
            v2["role"].as_str(),
            v2["roleAssigned"].as_bool(),
            v2["syncLocked"].as_bool()
        ),
        (Some("video"), Some(false), Some(true))
    );
    let error = ops(
        &mut runtime,
        json!([{"op": "set-track", "track": "V2", "role": "two\nlines"}]),
    )
    .unwrap_err();
    assert_eq!(error.code, "INVALID_EDIT");
    let error = ops(
        &mut runtime,
        json!([{"op": "set-track", "track": "V9", "syncLocked": false}]),
    )
    .unwrap_err();
    assert_eq!(error.code, "EDIT_REJECTED", "{error:?}");
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn the_rust_set_track_writes_as_the_json_one_reads() {
    let built = EditOp::SetTrack {
        track: "V1".into(),
        sequence: None,
        enabled: None,
        locked: Some(true),
        muted: None,
        role: None,
        sync_locked: None,
    };
    let written = serde_json::to_value(&built).unwrap();
    assert_eq!(
        written,
        json!({"op": "set-track", "track": "V1", "locked": true})
    );
    let read: EditOp = serde_json::from_value(written).unwrap();
    assert_eq!(read, built);
    let synced: EditOp =
        serde_json::from_value(json!({"op": "set-track", "track": "V1", "syncLocked": false}))
            .unwrap();
    assert_eq!(
        synced,
        EditOp::SetTrack {
            track: "V1".into(),
            sequence: None,
            enabled: None,
            locked: None,
            muted: None,
            role: None,
            sync_locked: Some(false),
        }
    );
    // Both lower to the same edit.
    let (mut runtime, directory) = project("set-track-forms");
    let built = runtime
        .apply_ops_request("ui", "built", vec![built], 0)
        .unwrap();
    let (mut other, other_directory) = project("set-track-forms-read");
    let read = other
        .apply_ops_request("ui", "read", vec![read], 0)
        .unwrap();
    assert_eq!(built.document, read.document);
    drop((runtime, other));
    let _ = fs::remove_dir_all(directory);
    let _ = fs::remove_dir_all(other_directory);
}

#[test]
fn ripple_edits_move_sync_locked_tracks_and_leave_the_others() {
    // Where c2, the title on V2, a2 on A1 and a3 on A2 start once c1 and
    // its sound are ripple-deleted, closing 2 s on V1 and A1.
    let starts = |synced: bool| {
        let (mut runtime, directory) = project("ripple-sync");
        ops(
            &mut runtime,
            json!([
                {"op": "add-track", "kind": "audio", "id": "A2"},
                {"op": "overwrite", "media": "m1", "sourceIn": 0, "frames": 24, "at": 72,
                 "tracks": ["A2"], "ids": ["a3"]},
                {"op": "set-track", "track": "A2", "syncLocked": synced}
            ]),
        )
        .unwrap();
        let state = ops(
            &mut runtime,
            json!([{"op": "ripple-delete", "clips": ["c1", "a1"]}]),
        )
        .unwrap();
        drop(runtime);
        let _ = fs::remove_dir_all(directory);
        ["c2", "t1", "a2", "a3"].map(|id| {
            state.timeline["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|track| track["clips"].as_array().unwrap())
                .find(|clip| clip["id"] == id)
                .and_then(|clip| clip["timelineStart"].as_f64())
                .unwrap()
        })
    };
    // Sync-locked, A2 and the title on V2 follow the edit.
    assert_eq!(starts(true), [0.0, 1.0, 1.0, 1.0]);
    // A2 out of sync lock stays where it is; the others still follow.
    assert_eq!(starts(false), [0.0, 1.0, 1.0, 3.0]);
}

// ---- renders ---------------------------------------------------------------

fn render(
    runtime: &mut Runtime,
    options: Value,
) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
    runtime.start_render_with(serde_json::from_value(options).unwrap())
}

#[test]
fn the_render_list_shows_queued_running_and_finished_jobs() {
    let (mut runtime, directory) = project("render-list");
    let (first, _, _) = render(&mut runtime, json!({"quality": "full", "name": "Full"})).unwrap();
    let (second, _, _) = render(&mut runtime, json!({"quality": "review"})).unwrap();
    let (third, _, _) =
        render(&mut runtime, json!({"quality": "master", "name": "Master"})).unwrap();
    runtime
        .update_job(&first.job_id, "completed", None, None, None)
        .unwrap();
    runtime
        .update_job(&second.job_id, "running", None, None, Some(0.25))
        .unwrap();
    let RuntimeResponse::RenderJobs(jobs) = runtime.execute(RuntimeRequest::ListRenders).unwrap()
    else {
        panic!("a list of jobs");
    };
    let listed: Vec<_> = jobs
        .iter()
        .map(|job| {
            (
                job.job_id.as_str(),
                job.status.as_str(),
                job.name.as_deref(),
                job.quality.as_deref(),
                job.codec.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (
                first.job_id.as_str(),
                "completed",
                Some("Full"),
                Some("full"),
                Some("h264")
            ),
            (
                second.job_id.as_str(),
                "running",
                None,
                Some("review"),
                Some("h264")
            ),
            (
                third.job_id.as_str(),
                "queued",
                Some("Master"),
                Some("master"),
                Some("prores_hq")
            ),
        ]
    );
    assert_eq!(jobs[1].progress, 0.25);
    assert_eq!(jobs[0].progress, 1.0);
    // On the wire: `jobs`, each in the job's own shape.
    let wire = crate::response_value("list", Ok(RuntimeResponse::RenderJobs(jobs)));
    assert_eq!(wire["jobs"][2]["codec"], "prores_hq");
    assert_eq!(wire["jobs"][2]["quality"], "master");
    assert_eq!(wire["jobs"][2]["name"], "Master");
    // Jobs survive a restart with what they were asked for; the ones still
    // queued or running fail as abandoned.
    drop(runtime);
    let reopened = Runtime::open(&directory).unwrap();
    let jobs = reopened.render_jobs();
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[2].codec.as_deref(), Some("prores_hq"));
    assert_eq!(jobs[2].status, "failed");
    drop(reopened);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn render_options_choose_the_codec_container_and_file() {
    let (mut runtime, directory) = project("render-options");
    let real = directory.canonicalize().unwrap();
    // A master is ProRes 422 HQ in QuickTime, named by its job without a name.
    let (job, _, output) = render(&mut runtime, json!({"quality": "master"})).unwrap();
    assert_eq!(output, directory.join(format!("{}.mov", job.job_id)));
    assert_eq!(job.output_path.as_deref(), output.to_str());
    // A name names the file in the folder chosen; folders inside the
    // project are made.
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/atlas_v7/", "name": "atlas_30s_v7 master"}),
    )
    .unwrap();
    assert_eq!(
        output,
        directory.join("deliveries/atlas_v7/atlas_30s_v7 master.mov")
    );
    assert!(directory.join("deliveries/atlas_v7").is_dir());
    // A file named outright, and characters a file name cannot hold.
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "review", "output": "deliveries/review.mp4", "name": "Review: v7"}),
    )
    .unwrap();
    assert_eq!(output, directory.join("deliveries/review.mp4"));
    let (job, _, output) = render(
        &mut runtime,
        json!({"quality": "full", "codec": "hevc", "output": "deliveries", "name": "a/b: c"}),
    )
    .unwrap();
    assert_eq!(output, directory.join("deliveries/a-b- c.mp4"));
    assert_eq!(job.name.as_deref(), Some("a/b: c"));
    // An absolute folder the user chose outside the project.
    let chosen = std::env::temp_dir().join(format!("kerf-e1-chosen-{}", unique_id()));
    fs::create_dir_all(&chosen).unwrap();
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "full", "codec": "prores_hq", "output": chosen, "name": "Master"}),
    )
    .unwrap();
    assert_eq!(output, chosen.join("Master.mov"));
    // An absolute path inside the project through the real path counts as
    // inside it.
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "preview", "output": real.join("previews"), "name": "p"}),
    )
    .unwrap();
    assert_eq!(output, real.join("previews/p.mp4"));

    let refused = |runtime: &mut Runtime, options: Value| {
        let error = render(runtime, options.clone()).unwrap_err();
        (error.code, error.message)
    };
    for (options, code) in [
        (json!({"quality": "best"}), "INVALID_RENDER_QUALITY"),
        (
            json!({"quality": "full", "codec": "vp9"}),
            "INVALID_RENDER_CODEC",
        ),
        (
            json!({"quality": "preview", "codec": "prores_hq"}),
            "INVALID_RENDER_CODEC",
        ),
        (
            json!({"quality": "review", "codec": "hevc"}),
            "INVALID_RENDER_CODEC",
        ),
        (
            json!({"quality": "full", "name": "two\nlines"}),
            "INVALID_RENDER_NAME",
        ),
        (
            json!({"quality": "master", "name": "..."}),
            "INVALID_RENDER_OUTPUT",
        ),
    ] {
        assert_eq!(refused(&mut runtime, options.clone()).0, code, "{options}");
    }
    assert_eq!(
        runtime.render_jobs().len(),
        6,
        "refused renders queue nothing"
    );
    let _ = fs::remove_dir_all(chosen);
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn render_outputs_outside_the_project_are_refused() {
    let (mut runtime, directory) = project("render-outside");
    let missing = std::env::temp_dir().join(format!("kerf-e1-missing-{}", unique_id()));
    let refused = |runtime: &mut Runtime, output: Value| {
        let error = render(runtime, json!({"quality": "master", "output": output})).unwrap_err();
        assert_eq!(error.code, "INVALID_RENDER_OUTPUT", "{output}: {error:?}");
        error.message
    };
    assert!(refused(&mut runtime, json!("../elsewhere/")).contains("climbs out"));
    assert!(refused(&mut runtime, json!("deliveries/../../x.mov")).contains("climbs out"));
    assert!(refused(&mut runtime, json!(missing.join("out/"))).contains("does not exist"));
    assert!(refused(&mut runtime, json!(".git/")).contains("hidden folder"));
    assert!(refused(&mut runtime, json!("review/.cache/m.mov")).contains("hidden folder"));
    // The container decides the extension.
    assert!(refused(&mut runtime, json!("deliveries/master.mp4")).contains(".mov file"));
    assert!(refused(&mut runtime, json!(" ")).contains("empty"));
    assert!(!missing.exists(), "nothing is made outside the project");
    #[cfg(unix)]
    {
        // A link inside the project that leads out of it.
        let outside = std::env::temp_dir().join(format!("kerf-e1-outside-{}", unique_id()));
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, directory.join("exports")).unwrap();
        assert!(refused(&mut runtime, json!("exports/")).contains("leads outside"));
        // Folders that do not exist yet behind the link are looked at
        // before any is made, so nothing appears outside.
        assert!(refused(&mut runtime, json!("exports/new/sub/")).contains("leads outside"));
        assert!(refused(&mut runtime, json!("exports/new/m.mov")).contains("leads outside"));
        assert!(
            !outside.join("new").exists(),
            "nothing is made through a link"
        );
        // A link that leads nowhere is refused, not followed.
        std::os::unix::fs::symlink(outside.join("gone"), directory.join("dangling")).unwrap();
        assert!(refused(&mut runtime, json!("dangling/new/")).contains("leads outside"));
        assert!(!outside.join("gone").exists());
        let _ = fs::remove_dir_all(outside);
    }
    // Two queued renders never write one file.
    render(&mut runtime, json!({"quality": "master", "name": "Master"})).unwrap();
    let error = render(&mut runtime, json!({"quality": "master", "name": "Master"})).unwrap_err();
    assert_eq!(error.code, "RENDER_OUTPUT_BUSY");
    assert_eq!(runtime.render_jobs().len(), 1);
    // A busy render is refused before any folder is made for it.
    render(
        &mut runtime,
        json!({"quality": "master", "output": "busy/", "name": "B"}),
    )
    .unwrap();
    fs::remove_dir_all(directory.join("busy")).unwrap();
    let error = render(
        &mut runtime,
        json!({"quality": "master", "output": "busy/", "name": "B"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "RENDER_OUTPUT_BUSY");
    assert!(
        !directory.join("busy").exists(),
        "a refused render makes nothing"
    );
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn render_requests_carry_their_options_on_the_wire() {
    let request: RuntimeRequest = serde_json::from_value(json!({
        "operation": "start_render", "quality": "master", "codec": "prores_hq",
        "output": "deliveries/", "name": "Master", "sequence": "s1"
    }))
    .unwrap();
    let RuntimeRequest::StartRender {
        codec,
        output,
        name,
        ..
    } = &request
    else {
        panic!("a render request");
    };
    assert_eq!(
        (codec.as_deref(), output.as_deref(), name.as_deref()),
        (Some("prores_hq"), Some("deliveries/"), Some("Master"))
    );
    let listed: RuntimeRequest =
        serde_json::from_value(json!({"operation": "list_renders"})).unwrap();
    assert!(matches!(listed, RuntimeRequest::ListRenders));
    // A job recorded before jobs kept their options still reads.
    let job: RenderJob = serde_json::from_value(json!({
        "jobId": "render-1", "status": "completed", "outputUrl": null,
        "outputPath": "/p/render-1.mp4", "error": null
    }))
    .unwrap();
    assert_eq!(
        (
            job.quality.as_deref(),
            job.codec.as_deref(),
            job.name.as_deref()
        ),
        (None, None, None)
    );
    let written = serde_json::to_value(&job).unwrap();
    assert!(written.get("quality").is_none() && written.get("codec").is_none());
}

#[test]
fn finished_renders_are_never_written_over() {
    let (mut runtime, directory) = project("render-again");
    let options = json!({"quality": "master", "output": "deliveries/", "name": "Master"});
    let (first, _, output) = render(&mut runtime, options.clone()).unwrap();
    assert_eq!(output, directory.join("deliveries/Master.mov"));
    runtime
        .update_job(&first.job_id, "completed", None, None, None)
        .unwrap();
    // Exporting `Master` again takes the next free name, so the finished
    // job still describes its file.
    let (second, _, output) = render(&mut runtime, options.clone()).unwrap();
    assert_eq!(output, directory.join("deliveries/Master 2.mov"));
    assert_eq!(second.output_path.as_deref(), output.to_str());
    assert_eq!(second.name.as_deref(), Some("Master"));
    assert_eq!(
        runtime
            .render_job(&first.job_id)
            .unwrap()
            .output_path
            .as_deref(),
        directory.join("deliveries/Master.mov").to_str()
    );
    // A file already on disk is a name taken too.
    fs::write(directory.join("deliveries/Taken.mov"), b"not a render").unwrap();
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/", "name": "Taken"}),
    )
    .unwrap();
    assert_eq!(output, directory.join("deliveries/Taken 2.mov"));
    assert_eq!(
        fs::read(directory.join("deliveries/Taken.mov")).unwrap(),
        b"not a render"
    );
    // A file named outright is refused rather than renamed.
    let error = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/Master.mov"}),
    )
    .unwrap_err();
    assert_eq!(error.code, "RENDER_OUTPUT_EXISTS");
    // A job that failed without leaving a file does not hold its name.
    let (failed, _, _) = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/", "name": "Retry"}),
    )
    .unwrap();
    runtime
        .update_job(&failed.job_id, "failed", Some("boom".into()), None, None)
        .unwrap();
    let (_, _, output) = render(
        &mut runtime,
        json!({"quality": "master", "output": "deliveries/", "name": "Retry"}),
    )
    .unwrap();
    assert_eq!(output, directory.join("deliveries/Retry.mov"));
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn render_jobs_list_in_the_order_they_were_made_across_runs() {
    let (mut runtime, directory) = project("render-order");
    // Each kerfd run counts its sequence from 0, so a job from an earlier
    // run can carry a longer sequence than a newer one; one run on a
    // coarse clock can make two jobs at one time.
    let ids = [
        "render-1800000000000000-1a",
        "render-1800000000000001-3",
        "render-1800000000000002-9",
        "render-1800000000000002-10",
        "render-18000000000000030-0",
    ];
    for (index, id) in ids.iter().rev().enumerate() {
        let job: RenderJob = serde_json::from_value(json!({
            "jobId": id, "status": if index % 2 == 0 { "completed" } else { "queued" },
            "outputUrl": null, "outputPath": format!("/p/{id}.mov"), "error": null
        }))
        .unwrap();
        runtime.state.jobs.insert((*id).to_owned(), job);
    }
    let listed: Vec<String> = runtime
        .render_jobs()
        .into_iter()
        .map(|job| job.job_id)
        .collect();
    assert_eq!(listed, ids);
    // The oldest finished jobs are the ones dropped: of one time, `-9`
    // came before `-10`.
    let mut jobs: HashMap<String, RenderJob> = HashMap::new();
    for seq in 9..=9 + MAX_FINISHED_JOBS {
        let id = format!("render-1800000000000000-{seq:x}");
        let job: RenderJob = serde_json::from_value(json!({
            "jobId": id, "status": "completed", "outputUrl": null,
            "outputPath": null, "error": null
        }))
        .unwrap();
        jobs.insert(id, job);
    }
    bound_jobs(&mut jobs);
    assert_eq!(jobs.len(), MAX_FINISHED_JOBS);
    assert!(!jobs.contains_key("render-1800000000000000-9"));
    assert!(jobs.contains_key("render-1800000000000000-10"));
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn a_job_that_finishes_keeps_the_finished_history_bounded() {
    let (mut runtime, directory) = project("render-bound-finish");
    for seq in 0..MAX_FINISHED_JOBS {
        let id = format!("render-1800000000000000-{seq:x}");
        let job: RenderJob = serde_json::from_value(json!({
            "jobId": id, "status": "completed", "outputUrl": null,
            "outputPath": null, "error": null
        }))
        .unwrap();
        runtime.state.jobs.insert(id, job);
    }
    let running = format!("render-1800000000000001-{:x}", MAX_FINISHED_JOBS);
    let job: RenderJob = serde_json::from_value(json!({
        "jobId": running, "status": "running", "outputUrl": null,
        "outputPath": null, "error": null
    }))
    .unwrap();
    runtime.state.jobs.insert(running.clone(), job);
    runtime
        .update_job(&running, "cancelled", None, None, None)
        .unwrap();
    let finished = runtime
        .render_jobs()
        .into_iter()
        .filter(|job| job_finished(&job.status))
        .count();
    assert_eq!(finished, MAX_FINISHED_JOBS);
    assert!(runtime.render_job(&running).is_ok());
    assert!(runtime.render_job("render-1800000000000000-0").is_err());
    drop(runtime);
    let _ = fs::remove_dir_all(directory);
}
