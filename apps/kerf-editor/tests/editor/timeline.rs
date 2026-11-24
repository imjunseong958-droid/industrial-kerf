//! The timeline: tracks and clips, the ruler and zoom, selection and tools, proposal bands, and the keys drawn on a clip.

use kerf_editor::model::{
    default_targets, proposal_changes, signal_keys, source_targets, toggle_target, ClipShow,
    GainKey, Interp, Proposal, ProposalBand, ProposalStatus, ProposalVersion, Snapshot,
    SnapshotShow, Timeline, TimelineShow, TrackKind, TrackShow,
};
use kerf_editor::prefs::TrackHeights;
use kerf_editor::store::MediaActivity;
use kerf_editor::theme::{layout, motion};
use kerf_editor::timeline::{
    activity_label, add_gain_key, band_line, extend, gain_at, gain_y, shown_version, signal_at,
    signal_points, stepped_height, strip_has_transport, watch_look, watched_clips, zoom_of_share,
    zoom_share, ClipWatch, StripFit, Tool, WatchLook, GAIN_CEILING, GAIN_FLOOR, TRACK_LEAST,
    TRACK_MOST, TRACK_STEPS,
};
use proptest::prelude::*;
use serde_json::json;
use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

fn timeline(value: serde_json::Value) -> Timeline {
    Timeline::from_value(&value)
}

fn program() -> Timeline {
    timeline(json!({
        "fps": 24.0,
        "duration": 10.0,
        "sources": [
            {"id": "m", "path": "media/shot.mp4", "duration": 30.0, "width": 1920, "height": 1080, "hasAudio": true},
            {"id": "s", "path": "media/tone.wav", "duration": 30.0, "hasAudio": true}
        ],
        "tracks": [
            {"id": "V1", "type": "video", "enabled": true, "clips": [
                {"id": "a", "timelineStart": 0.0, "duration": 4.0, "startFrame": 0, "durationFrames": 96, "enabled": true, "sourceId": "m", "in": 0.0, "out": 4.0},
                {"id": "b", "timelineStart": 4.0, "duration": 6.0, "startFrame": 96, "durationFrames": 144, "enabled": true, "sourceId": "m", "in": 4.0, "out": 10.0}
            ]},
            {"id": "V2", "type": "video", "enabled": true, "locked": true, "clips": []},
            {"id": "A1", "type": "audio", "enabled": true, "clips": [
                {"id": "s1", "timelineStart": 0.0, "duration": 10.0, "startFrame": 0, "durationFrames": 240, "enabled": true, "sourceId": "s", "in": 0.0, "out": 10.0}
            ]},
            {"id": "A2", "type": "audio", "enabled": true, "role": "Room Tone", "clips": []}
        ]
    }))
}

#[test]
fn a_proposal_reads_as_what_it_moves_trims_adds_and_takes_out() {
    let current = program();
    // b moves a second later and is trimmed; a goes; c comes on V2.
    let proposed = timeline(json!({
        "fps": 24.0,
        "tracks": [
            {"id": "V1", "type": "video", "enabled": true, "clips": [
                {"id": "b", "timelineStart": 5.0, "duration": 5.0, "startFrame": 120, "durationFrames": 120, "enabled": true, "sourceId": "m", "in": 4.0, "out": 9.0}
            ]},
            {"id": "V2", "type": "video", "enabled": true, "clips": [
                {"id": "c", "timelineStart": 12.0, "duration": 2.0, "startFrame": 288, "durationFrames": 48, "enabled": true}
            ]},
            {"id": "A1", "type": "audio", "enabled": true, "clips": [
                {"id": "s1", "timelineStart": 0.0, "duration": 10.0, "startFrame": 0, "durationFrames": 240, "enabled": true, "sourceId": "s", "in": 0.0, "out": 10.0}
            ]}
        ]
    }));
    let changes = proposal_changes(&current, &proposed);
    let by = |id: &str| changes.iter().find(|change| change.clip == id).unwrap();
    assert!(by("b").moved && by("b").trimmed && !by("b").retracked);
    assert_eq!(by("b").old, Some((4.0, 10.0)));
    assert!(by("a").removed() && by("a").leaves_ghost());
    assert!(by("c").added() && !by("c").leaves_ghost());
    // The sound it leaves as it was is no change.
    assert!(changes.iter().all(|change| change.clip != "s1"));

    let band = ProposalBand::of("p1", "Kerf", &current, &proposed).unwrap();
    assert_eq!((band.start, band.end), (0.0, 14.0));
    assert_eq!(band.author, "Kerf");
    for track in ["V1", "V2"] {
        assert!(band.tracks.iter().any(|known| known == track), "{track}");
    }
    assert!(!band.tracks.iter().any(|known| known == "A1"));
    // Ghosts stand where the clips it moves or takes out are now.
    let mut ghosts = band.ghosts.clone();
    ghosts.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(
        ghosts,
        [("V1".to_owned(), 0.0, 4.0), ("V1".to_owned(), 4.0, 10.0)]
    );
    // Nothing changed: no band.
    assert_eq!(ProposalBand::of("p2", "Kerf", &current, &current), None);
}

proptest! {

    /// The keys kept of a drawn line start and end where it does, go in
    /// order, and every frame drawn is within reach of the line through them.
    #[test]
    fn a_drawn_signal_keeps_its_shape_in_fewer_keys(
        values in prop::collection::vec(0.0f64..=1.0, 1..80),
        start in 0i64..50,
    ) {
        let drawn: BTreeMap<i64, f64> = values
            .iter()
            .enumerate()
            .map(|(index, value)| (start + index as i64, *value))
            .collect();
        let keys = signal_keys(&drawn, 0.02);
        prop_assert!(!keys.is_empty() && keys.len() <= drawn.len());
        let frames: Vec<i64> = keys.iter().map(|key| key["at"].as_i64().unwrap()).collect();
        prop_assert_eq!(frames[0], start);
        prop_assert_eq!(*frames.last().unwrap(), start + values.len() as i64 - 1);
        prop_assert!(frames.windows(2).all(|pair| pair[0] < pair[1]));
        for key in &keys {
            prop_assert_eq!(key["interp"].as_str(), Some("linear"));
        }
        for (frame, value) in &drawn {
            let after = frames.iter().position(|kept| kept >= frame).unwrap();
            let line = if frames[after] == *frame || after == 0 {
                keys[after]["value"].as_f64().unwrap()
            } else {
                let (a, b) = (&keys[after - 1], &keys[after]);
                let (fa, fb) = (frames[after - 1] as f64, frames[after] as f64);
                let (va, vb) = (a["value"].as_f64().unwrap(), b["value"].as_f64().unwrap());
                va + (vb - va) * (*frame as f64 - fa) / (fb - fa)
            };
            prop_assert!((line - value).abs() <= 0.02 + 0.002, "{frame}: {line} vs {value}");
        }
    }

    /// The rubber band's height and the gain it reads are one another's.
    #[test]
    fn the_rubber_band_reads_back_the_gain_it_draws(db in GAIN_FLOOR..=GAIN_CEILING, top in -50.0f32..500.0, tall in 12.0f32..160.0) {
        let y = gain_y(db, top, tall);
        prop_assert!(y >= top + 2.0 - 1e-3 && y <= top + tall - 2.0 + 1e-3);
        let read = gain_at(y, top, tall);
        prop_assert!((read - db).abs() <= 0.06, "{db} -> {y} -> {read}");
    }

    /// Louder is higher.
    #[test]
    fn a_louder_band_stands_higher(a in GAIN_FLOOR..=GAIN_CEILING, b in GAIN_FLOOR..=GAIN_CEILING) {
        prop_assume!(a < b);
        prop_assert!(gain_y(a, 0.0, 40.0) >= gain_y(b, 0.0, 40.0));
    }

    /// The slider's place and the zoom it gives are one another's, past its detent.
    #[test]
    fn the_zoom_slider_reads_back_its_zoom(share in 0.03f32..=1.0, fit in 1.0f32..200.0, reach in 1.5f32..400.0) {
        let most = fit * reach;
        let pps = zoom_of_share(share, fit, most);
        prop_assert!(pps >= fit && pps <= most * 1.0001);
        prop_assert!((zoom_share(pps, fit, most) - share).abs() < 1e-3);
    }

    /// Stepping a height stays on the ladder, goes the way it is asked, and
    /// a step back up undoes a step down.
    #[test]
    fn track_heights_step_along_the_ladder(height in TRACK_LEAST..=TRACK_MOST, steps in -20i32..20) {
        let stepped = stepped_height(height, steps);
        prop_assert!((TRACK_LEAST..=TRACK_MOST).contains(&stepped));
        if steps != 0 {
            prop_assert!(TRACK_STEPS.contains(&stepped) || stepped == height);
        }
        if steps > 0 { prop_assert!(stepped >= height); }
        if steps < 0 { prop_assert!(stepped <= height); }
        let on = TRACK_STEPS[(height as usize) % TRACK_STEPS.len()];
        if on > TRACK_LEAST {
            prop_assert_eq!(stepped_height(stepped_height(on, -1), 1), on);
        }
    }

    /// Whatever is clicked, the selection holds each clip once, and the
    /// one clicked leads it unless ⌘ let it go.
    #[test]
    fn the_last_clicked_clip_leads(clicks in prop::collection::vec((0usize..6, any::<bool>()), 1..30)) {
        let mut selection: Vec<String> = Vec::new();
        for (clip, toggle) in clicks {
            let id = format!("c{clip}");
            let had = selection.contains(&id);
            extend(&mut selection, &id, toggle);
            let mut seen = selection.clone();
            seen.sort();
            seen.dedup();
            prop_assert_eq!(seen.len(), selection.len());
            if toggle && had {
                prop_assert!(!selection.contains(&id));
            } else {
                prop_assert_eq!(selection.first(), Some(&id));
            }
        }
    }

    /// Whatever target chips are pressed, a source goes on at most one
    /// track of each kind, one the person targeted and not locked; and the
    /// chip pressed last on a kind is the one it goes on.
    #[test]
    fn a_source_goes_on_one_targeted_track_of_each_kind(
        presses in prop::collection::vec(0usize..5, 0..24),
    ) {
        let program = two_of_each();
        let both = program.source("m").unwrap();
        let ids = ["V1", "V2", "V3", "A1", "A2"];
        let mut targets = default_targets(&program);
        let mut last: Vec<(TrackKind, &str, bool)> = Vec::new();
        for press in presses {
            let id = ids[press];
            let on = !targets.iter().any(|known| known == id);
            targets = toggle_target(&program, &targets, id);
            let kind = program.tracks.iter().find(|track| track.id == id).unwrap().kind;
            last.retain(|(known, _, _)| *known != kind);
            last.push((kind, id, on));
            for kind in [TrackKind::Video, TrackKind::Audio] {
                let of_kind = targets
                    .iter()
                    .filter(|id| program.tracks.iter().any(|track| &track.id == *id && track.kind == kind))
                    .count();
                prop_assert!(of_kind <= 1, "{targets:?}");
            }
            let landing = source_targets(&program, both, Some(&targets));
            prop_assert!(landing.len() <= 2);
            for id in &landing {
                prop_assert!(targets.contains(id));
                prop_assert!(id != "V3", "a locked track never takes a source");
            }
            for (kind, id, on) in &last {
                let lands = landing.iter().find(|landed| {
                    program.tracks.iter().any(|track| &track.id == *landed && track.kind == *kind)
                });
                if *on && *id != "V3" {
                    prop_assert_eq!(lands.map(String::as_str), Some(*id));
                } else if !*on {
                    prop_assert_eq!(lands, None);
                }
            }
        }
    }

    /// Where the pointer draws a signal over its clip's box is the frame and
    /// value the curve is drawn at there: the drawing reads back, wherever
    /// the box stands (moved with the timeline, the pointer on it reads the
    /// same).
    #[test]
    fn a_signal_drawn_reads_back_where_it_is_drawn(
        frames in 1i64..400,
        at in 0.0f64..=1.0,
        value in 0.0f64..=1.0,
        left in -200.0f32..900.0,
        top in -100.0f32..700.0,
        across in 20.0f32..1200.0,
        tall in 12.0f32..160.0,
        shift in -120.0f32..120.0,
    ) {
        let frame = ((frames - 1) as f64 * at).round() as i64;
        // As the drawn line is painted: across the curve's box, 2 points in.
        let last = (frames - 1).max(1) as f32;
        let (h, curve_top) = (tall - 4.0, top + 2.0);
        let x = left + across * frame as f32 / last;
        let y = curve_top + h - h * value as f32;
        let (read_frame, read) = signal_at(x, y, (left, top, across, tall), frames).unwrap();
        prop_assert_eq!(read_frame, frame);
        prop_assert!((read - value).abs() < 1e-3, "{value} read as {read}");
        let moved = signal_at(x, y + shift, (left, top + shift, across, tall), frames).unwrap();
        prop_assert_eq!(moved.0, frame);
        prop_assert!((moved.1 - value).abs() < 1e-3);
    }

    /// Option-click adds a key among a band's keys in frame order, the
    /// keys there keep their values, and the band then runs through it.
    #[test]
    fn a_key_added_to_the_band_is_drawn_where_it_was_clicked(
        known in prop::collection::btree_map(0i64..96, GAIN_FLOOR..=GAIN_CEILING, 0..5),
        frame in 0i64..96,
        level in GAIN_FLOOR..=GAIN_CEILING,
    ) {
        let fps = 24.0;
        let keyed = !known.is_empty();
        let mut keys: Vec<GainKey> = if keyed {
            known
                .iter()
                .map(|(frame, value)| GainKey { at: *frame as f64 / fps, frame: *frame, value: *value, interp: Interp::Linear })
                .collect()
        } else {
            vec![GainKey { at: 0.0, frame: 0, value: -6.0, interp: Interp::Linear }]
        };
        let index = add_gain_key(&mut keys, keyed, frame, fps, level);
        prop_assert_eq!(keys[index].frame, frame);
        prop_assert!(keys.windows(2).all(|pair| pair[0].frame < pair[1].frame));
        let expected = known.get(&frame).copied().unwrap_or(level);
        prop_assert_eq!(keys[index].value, expected);
        for (frame, value) in &known {
            let kept = keys.iter().find(|key| key.frame == *frame).unwrap();
            prop_assert_eq!(kept.value, *value);
        }
        prop_assert_eq!(keys.len(), known.len() + usize::from(!known.contains_key(&frame)));
        // Drawn from these keys (as the drag draws it), the band has a
        // diamond on the new key at the level it was clicked.
        let values: Vec<f64> = keys.iter().map(|key| key.value).collect();
        let pps = 10.0;
        let (points, drawn_keyed) = band_line(&keys, &values, true, 0.0, 96.0 / 24.0 * pps, 0.0, 40.0, pps);
        prop_assert!(drawn_keyed);
        prop_assert_eq!(points.len(), keys.len() + 2);
        let (x, y) = points[index + 1];
        prop_assert!((x - frame as f32 / 24.0 * pps).abs() < 1e-3);
        prop_assert!((y - gain_y(expected, 0.0, 40.0)).abs() < 1e-3);
    }
}

/// A program with three picture tracks (V3 locked) and two sound tracks,
/// and a media with both.
fn two_of_each() -> Timeline {
    timeline(json!({
        "fps": 24.0,
        "duration": 4.0,
        "sources": [
            {"id": "m", "path": "media/shot.mp4", "duration": 30.0, "width": 1920, "height": 1080, "hasAudio": true}
        ],
        "tracks": [
            {"id": "V1", "type": "video", "enabled": true, "clips": []},
            {"id": "V2", "type": "video", "enabled": true, "clips": []},
            {"id": "V3", "type": "video", "enabled": true, "locked": true, "clips": []},
            {"id": "A1", "type": "audio", "enabled": true, "clips": []},
            {"id": "A2", "type": "audio", "enabled": true, "clips": []}
        ]
    }))
}

#[test]
fn targeting_a_second_track_of_a_kind_moves_the_source_there() {
    let program = two_of_each();
    let media = program.source("m").unwrap();
    let targets = default_targets(&program);
    assert_eq!(targets, ["V1", "A1"]);
    // V2's chip: the picture goes on V2 alone, the sound still on A1.
    let targets = toggle_target(&program, &targets, "V2");
    assert_eq!(targets, ["A1", "V2"]);
    assert_eq!(
        source_targets(&program, media, Some(&targets)),
        ["V2", "A1"]
    );
    // V2's chip again: no picture goes in.
    let targets = toggle_target(&program, &targets, "V2");
    assert_eq!(source_targets(&program, media, Some(&targets)), ["A1"]);
    // Chosen any way at all, only the first targeted unlocked track of each
    // kind takes the source.
    let chosen: Vec<String> = ["V3", "V2", "V1", "A2", "A1"].map(String::from).to_vec();
    assert_eq!(source_targets(&program, media, Some(&chosen)), ["V1", "A1"]);
}

#[test]
fn shift_on_a_chosen_clip_makes_it_lead() {
    let mut selection = vec!["a".to_owned()];
    extend(&mut selection, "b", false);
    assert_eq!(selection, ["b", "a"]);
    extend(&mut selection, "a", false);
    assert_eq!(selection, ["a", "b"]);
    extend(&mut selection, "a", true);
    assert_eq!(selection, ["b"]);
}

#[test]
fn every_track_steps_and_new_ones_follow() {
    let rows = vec![
        ("V1".to_owned(), TrackKind::Video),
        ("A1".to_owned(), TrackKind::Audio),
    ];
    let mut heights = TrackHeights::default();
    heights.set("A1", Some(20.0));
    heights.step(&rows, 1);
    assert_eq!(heights.height("A1", TrackKind::Audio), 24.0);
    let video = heights.height("V1", TrackKind::Video);
    assert!(video > kerf_editor::theme::layout::TRACK_LANE);
    // A track added since takes the height the rest stepped to.
    assert_eq!(heights.height("V9", TrackKind::Video), video);
    heights.set_all(Some(500.0));
    assert_eq!(heights.height("A1", TrackKind::Audio), TRACK_MOST);
    heights.set_all(None);
    assert_eq!(
        heights.height("A1", TrackKind::Audio),
        kerf_editor::theme::layout::TRACK_LANE
    );
    // Kept with the project's settings as JSON.
    let kept = serde_json::to_value(&heights).unwrap();
    assert_eq!(
        serde_json::from_value::<TrackHeights>(kept).unwrap(),
        heights
    );
}

#[test]
fn source_edits_land_on_the_targets_of_the_kinds_the_media_has() {
    let program = program();
    let picture = program.source("m").unwrap();
    let sound = program.source("s").unwrap();
    // None chosen: the first unlocked track of each kind it has.
    assert_eq!(source_targets(&program, picture, None), ["V1", "A1"]);
    assert_eq!(source_targets(&program, sound, None), ["A1"]);
    // Chosen: only those, of its kinds, never a locked one.
    let chosen = ["V2".to_owned(), "A2".to_owned()];
    assert_eq!(source_targets(&program, picture, Some(&chosen)), ["A2"]);
    let chosen = ["V1".to_owned()];
    assert_eq!(
        source_targets(&program, sound, Some(&chosen)),
        Vec::<String>::new()
    );
}

#[test]
fn clips_read_their_gain_speed_and_title() {
    let clip = |props: serde_json::Value, extra: serde_json::Value| {
        let mut value = json!({"id": "x", "duration": 2.0, "durationFrames": 48, "props": props});
        if let (Some(value), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
            value.extend(extra.clone());
        }
        let tl = timeline(
            json!({"fps": 24.0, "tracks": [{"id": "A1", "type": "audio", "clips": [value]}]}),
        );
        tl.tracks[0].clips[0].clone()
    };
    let level = clip(json!({"gain": -6.0}), json!({}));
    let (keys, keyed) = level.gain_keys();
    assert!(!keyed);
    assert_eq!(keys[0].value, -6.0);
    let keyed_clip = clip(
        json!({"gain": [
            {"at": 1.0, "frame": 24, "value": -12.0, "interp": "linear"},
            {"at": 0.0, "frame": 0, "value": 0.0, "interp": "hold"}
        ]}),
        json!({}),
    );
    let (keys, keyed) = keyed_clip.gain_keys();
    assert!(keyed);
    assert_eq!(
        keys.iter().map(|key| key.frame).collect::<Vec<_>>(),
        [0, 24]
    );
    assert_eq!(keys[0].interp, Interp::Hold);
    assert_eq!(clip(json!({}), json!({"speed": 0.5})).secondary(), "50%");
    assert_eq!(clip(json!({}), json!({"speed": -2.0})).secondary(), "200%");
    assert_eq!(clip(json!({}), json!({})).secondary(), "");
    let title = clip(
        json!({"text": "Hello  world", "blend": "screen", "opacity": 0.5}),
        json!({"kind": "generator", "name": "Title"}),
    );
    assert_eq!(title.secondary(), "Hello world · Screen · 50%");
    let signal = clip(
        json!({}),
        json!({"kind": "generator", "signal": [0.0, 0.5, 1.0]}),
    );
    assert_eq!(signal.signal, [0.0, 0.5, 1.0]);
    let program = program();
    let room = program
        .tracks
        .iter()
        .find(|track| track.id == "A2")
        .unwrap();
    assert!(room.is_room_tone());
    assert!(!program.tracks[0].is_room_tone());
}

#[test]
fn a_signal_is_drawn_across_its_clip_low_to_high() {
    let points = signal_points(&[0.0, 1.0], (10.0, 0.0, 100.0, 40.0));
    assert_eq!(points, [(10.0, 40.0), (110.0, 0.0)]);
    // One value is a level the whole way.
    let level = signal_points(&[0.5], (0.0, 0.0, 50.0, 10.0));
    assert_eq!(level, [(0.0, 5.0), (50.0, 5.0)]);
    assert!(signal_points(&[], (0.0, 0.0, 10.0, 10.0)).is_empty());
}

#[test]
fn tools_are_the_registrys_and_say_what_they_take() {
    for tool in Tool::ALL {
        assert_eq!(Tool::from_command(tool.command()), Some(tool));
        assert!(kerf_editor::commands::catalog("kerf")
            .iter()
            .any(|command| command.id == tool.command()));
    }
    assert_eq!(Tool::default(), Tool::Selection);
    assert!(Tool::Selection.trims_edges() && Tool::Selection.drags_bodies());
    assert!(!Tool::Razor.trims_edges() && !Tool::Razor.drags_bodies());
    assert!(Tool::Ripple.trims_edges() && !Tool::Ripple.drags_bodies());
    assert!(Tool::Slip.drags_bodies() && !Tool::Slip.trims_edges());
    assert_eq!(Tool::Razor.cursor(false), gpui::CursorStyle::Crosshair);
    assert_eq!(Tool::Roll.cursor(true), gpui::CursorStyle::ResizeColumn);
    assert_eq!(Tool::Selection.cursor(false), gpui::CursorStyle::OpenHand);
}

#[test]
fn clips_say_how_their_media_stands() {
    assert_eq!(activity_label(&MediaActivity::Ready), None);
    assert_eq!(
        activity_label(&MediaActivity::Queued { ahead: Some(2) }).as_deref(),
        Some("Queued · 2 ahead")
    );
    assert_eq!(
        activity_label(&MediaActivity::Queued { ahead: None }).as_deref(),
        Some("Queued")
    );
    assert_eq!(
        activity_label(&MediaActivity::NotMade).as_deref(),
        Some("Not made")
    );
    assert_eq!(
        activity_label(&MediaActivity::Offline).as_deref(),
        Some("Offline")
    );
    let making = activity_label(&MediaActivity::Making {
        take: 2,
        since: Instant::now(),
    })
    .unwrap();
    assert_eq!(making, "Making take 2 · 0:00");
}

// ---------------------------------------------------------------- the strip's transport

#[test]
fn the_strip_carries_the_transport_only_while_the_picture_is_closed() {
    assert!(strip_has_transport(Some(false)));
    assert!(!strip_has_transport(Some(true)));
    // Until the window says, the stage has it.
    assert!(!strip_has_transport(None));
}

/// A strip as wide as the window's at 1200 points with the picture pane
/// closed, and its pieces: what the rest needs, the cut's length.
const STRIP_REST: f32 = 300.0;

const STRIP_LENGTH: f32 = 78.0;

#[test]
fn a_narrow_strip_lets_go_of_the_slider_then_the_ends_then_the_length() {
    let all = StripFit::ALL;
    let need = |fit: StripFit| STRIP_REST + fit.width(true, STRIP_LENGTH);
    // Room for all of it: all of it.
    assert_eq!(all.within(need(all), STRIP_REST, true, STRIP_LENGTH), all);
    // A point short: the slider goes, − and + stay.
    let fit = all.within(need(all) - 1.0, STRIP_REST, true, STRIP_LENGTH);
    assert_eq!(fit, StripFit::STEPS[1]);
    assert!(!fit.slider && fit.transport_ends && fit.length);
    // Then the transport's ends, then the cut's length.
    let fit = all.within(
        need(StripFit::STEPS[1]) - 1.0,
        STRIP_REST,
        true,
        STRIP_LENGTH,
    );
    assert_eq!(fit, StripFit::STEPS[2]);
    let fit = all.within(
        need(StripFit::STEPS[2]) - 1.0,
        STRIP_REST,
        true,
        STRIP_LENGTH,
    );
    assert_eq!(fit, StripFit::STEPS[3]);
    // Narrower than even that: all let go, and the rest is cut off.
    assert_eq!(
        all.within(10.0, STRIP_REST, true, STRIP_LENGTH),
        StripFit::STEPS[3]
    );
    // Without the transport its ends take no room: from the slider on, the
    // length is next.
    assert_eq!(
        StripFit::STEPS[1].width(false, STRIP_LENGTH),
        StripFit::STEPS[2].width(false, STRIP_LENGTH)
    );
    // Back with room: shown again only with some to spare.
    let tight = StripFit::STEPS[3].within(need(all), STRIP_REST, true, STRIP_LENGTH);
    assert_eq!(tight, StripFit::STEPS[1]);
    let spare = need(all) + layout::STRIP_REGROW;
    assert_eq!(
        StripFit::STEPS[3].within(spare, STRIP_REST, true, STRIP_LENGTH),
        all
    );
}

fn any_strip_fit() -> impl Strategy<Value = StripFit> {
    (0usize..4).prop_map(|step| StripFit::STEPS[step])
}

proptest! {
    #[test]
    fn a_strip_settles_on_the_most_that_fits(
        now in any_strip_fit(),
        room in 0f32..1200.0,
        more in 0f32..400.0,
        rest in 0f32..900.0,
        transport in any::<bool>(),
        length in 0f32..120.0,
    ) {
        let fit = now.within(room, rest, transport, length);
        let step = StripFit::STEPS.iter().position(|step| *step == fit).unwrap();
        // It fits, unless nothing does and all of it is let go.
        prop_assert!(rest + fit.width(transport, length) <= room || step == 3);
        // Nothing more would: what it shows more than now needs the spare too.
        let now_step = StripFit::STEPS.iter().position(|step| *step == now).unwrap();
        for (earlier, other) in StripFit::STEPS.iter().enumerate().take(step) {
            let spare = if earlier < now_step { layout::STRIP_REGROW } else { 0.0 };
            prop_assert!(rest + other.width(transport, length) + spare > room);
        }
        // Drawn again at the same width, it stays as it is: no showing and
        // hiding by turns.
        prop_assert_eq!(fit.within(room, rest, transport, length), fit);
        // More room never shows less.
        let wider = now.within(room + more, rest, transport, length);
        let wider_step = StripFit::STEPS.iter().position(|step| *step == wider).unwrap();
        prop_assert!(wider_step <= step);
    }
}

fn proposal(id: &str, versions: &[&str]) -> Proposal {
    Proposal {
        proposal_id: id.to_owned(),
        status: ProposalStatus::Pending,
        versions: versions
            .iter()
            .enumerate()
            .map(|(at, version)| ProposalVersion {
                version: at as u32 + 1,
                proposal_id: (*version).to_owned(),
                base_revision: 0,
                ..ProposalVersion::default()
            })
            .collect(),
        ..Proposal::default()
    }
}

#[test]
fn the_cut_kerf_builds_shows_its_newest_version_over_the_one_previewing() {
    // v3 is the newest; v1 plays as the preview, v2's card is pinned.
    let cut = proposal("v3", &["v1", "v2", "v3"]);
    assert_eq!(
        shown_version(&cut, Some("v3"), Some("v1"), Some("v2")),
        Some("v3")
    );
    // Once Kerf stops, what previews shows again, else the pinned card's.
    assert_eq!(
        shown_version(&cut, None, Some("v1"), Some("v2")),
        Some("v1")
    );
    assert_eq!(shown_version(&cut, None, None, Some("v2")), Some("v2"));
    assert_eq!(shown_version(&cut, None, None, None), None);
    // Another proposal built, previewed or pinned does not move this one.
    let other = proposal("w2", &["w1", "w2"]);
    assert_eq!(shown_version(&other, Some("v3"), Some("v1"), None), None);
    assert_eq!(
        shown_version(&other, Some("v3"), Some("w1"), None),
        Some("w1")
    );
}

proptest! {
    /// Whatever is asked, what is shown is a version of the proposal, and
    /// the first of building, previewing and pinned that is one.
    #[test]
    fn what_shows_is_always_a_version_of_its_proposal(
        versions in 1usize..5,
        building in proptest::option::of(0usize..8),
        previewing in proptest::option::of(0usize..8),
        pinned in proptest::option::of(0usize..8),
    ) {
        let ids: Vec<String> = (0..8).map(|at| format!("p{at}")).collect();
        let own: Vec<&str> = ids[..versions].iter().map(String::as_str).collect();
        let cut = proposal(own[versions - 1], &own);
        let pick = |at: Option<usize>| at.map(|at| ids[at].as_str());
        let shown = shown_version(&cut, pick(building), pick(previewing), pick(pinned));
        let first = [building, previewing, pinned]
            .into_iter()
            .flatten()
            .find(|at| *at < versions)
            .map(|at| ids[at].as_str());
        prop_assert_eq!(shown, first);
    }
}

// ------------------------------------------------------- watching changes

fn clip(id: &str, start: f64, duration: f64) -> serde_json::Value {
    json!({"id": id, "timelineStart": start, "duration": duration,
           "startFrame": (start * 24.0) as i64, "durationFrames": (duration * 24.0) as i64,
           "enabled": true})
}

fn reel_snapshot() -> Snapshot {
    let program = json!({"fps": 24.0, "duration": 12.0, "tracks": [
        {"id": "V1", "type": "video", "clips": [clip("a", 0.0, 4.0), clip("b", 4.0, 4.0), clip("d", 8.0, 4.0)]},
        {"id": "A1", "type": "audio", "clips": [clip("s", 0.0, 12.0)]}
    ]});
    // v1 trims a; v2 also takes b out and adds c.
    let first = json!({"fps": 24.0, "tracks": [
        {"id": "V1", "type": "video", "clips": [clip("a", 0.0, 3.0), clip("b", 4.0, 4.0), clip("d", 8.0, 4.0)]},
        {"id": "A1", "type": "audio", "clips": [clip("s", 0.0, 12.0)]}
    ]});
    let second = json!({"fps": 24.0, "tracks": [
        {"id": "V1", "type": "video", "clips": [clip("a", 0.0, 3.0), clip("c", 4.0, 4.0), clip("d", 8.0, 4.0)]},
        {"id": "A1", "type": "audio", "clips": [clip("s", 0.0, 12.0)]}
    ]});
    Snapshot::from_value(&json!({
        "timeline": program,
        "revision": 7,
        "proposals": [{"proposalId": "p2", "status": "pending", "version": 2,
            "versions": [{"version": 1, "proposalId": "p1"}, {"version": 2, "proposalId": "p2"}]}],
        "proposalTimelines": {"p1": first, "p2": second}
    }))
}

fn set(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn a_reel_watches_the_clips_its_proposal_changes() {
    let snapshot = reel_snapshot();
    // What the version played changes, as its card lists it: never d or s.
    assert_eq!(watched_clips(&snapshot, "p2"), set(&["a", "b", "c"]));
    assert_eq!(watched_clips(&snapshot, "p1"), set(&["a"]));
    assert!(watched_clips(&snapshot, "elsewhere").is_empty());
}

#[test]
fn watched_clips_rise_and_the_rest_dim_then_all_settle_back() {
    let stage = motion::STAGE;
    let t0 = Instant::now();
    let asked = Cell::new(0);
    let find = || {
        asked.set(asked.get() + 1);
        set(&["a"])
    };
    let mut watch = ClipWatch::default();
    assert_eq!(watch.amount_at(t0), 0.0);
    watch.follow_at(Some(("p1", 7)), find, false, t0);
    assert_eq!(watch.clips(), &set(&["a"]));
    // From the lanes, over STAGE, eased (more than half way at half time).
    assert_eq!(watch.amount_at(t0), 0.0);
    let half = watch.amount_at(t0 + stage / 2);
    assert!(half > 0.5 && half < 1.0, "{half}");
    assert_eq!(watch.amount_at(t0 + stage), 1.0);
    assert!(watch.running_at(t0 + stage / 2));
    assert!(!watch.running_at(t0 + stage));
    // The same reel at the same revision asks for its clips once.
    let again = || {
        asked.set(asked.get() + 1);
        set(&["a"])
    };
    watch.follow_at(Some(("p1", 7)), again, false, t0 + stage);
    assert_eq!(asked.get(), 1);
    let look = watch_look(watch.amount_at(t0 + stage), true, false);
    assert_eq!(look.lift, motion::CLIP_LIFT);
    // The reel ends: back down over STAGE, the clips kept until it has.
    let t1 = t0 + stage;
    watch.follow_at(None, HashSet::new, false, t1);
    assert_eq!(watch.amount_at(t1), 1.0);
    watch.follow_at(None, HashSet::new, false, t1 + stage / 2);
    assert!(watch.amount_at(t1 + stage / 2) < 0.5);
    assert_eq!(watch.clips(), &set(&["a"]));
    watch.follow_at(None, HashSet::new, false, t1 + stage);
    assert_eq!(watch.amount_at(t1 + stage), 0.0);
    assert!(watch.clips().is_empty());
    // Another reel's clips rise from their lanes anew.
    let t2 = t1 + stage * 2;
    watch.follow_at(Some(("p2", 7)), || set(&["a", "b", "c"]), false, t2);
    assert_eq!(watch.amount_at(t2), 0.0);
    // Under Reduce Motion, all at once: up as it starts, down as it ends.
    let t3 = t2 + Duration::from_millis(10);
    let mut still = ClipWatch::default();
    still.follow_at(Some(("p1", 7)), || set(&["a"]), true, t3);
    assert_eq!(still.amount_at(t3), 1.0);
    assert!(!still.running_at(t3));
    still.follow_at(None, HashSet::new, true, t3);
    assert_eq!(still.amount_at(t3), 0.0);
    assert!(still.clips().is_empty());
}

proptest! {
    #[test]
    fn a_watched_clip_rises_at_most_its_lift_and_the_rest_only_dim(
        amount in -0.5f32..1.5,
        audio in any::<bool>(),
    ) {
        let shown = amount.clamp(0.0, 1.0);
        let watched = watch_look(amount, true, audio);
        let other = watch_look(amount, false, audio);
        prop_assert!((0.0..=motion::CLIP_LIFT).contains(&watched.lift));
        // A sound clip stays on its lane, ringed; a picture's rises over its glow.
        if audio {
            prop_assert_eq!(watched.lift, 0.0);
            prop_assert_eq!(watched.glow, 0.0);
        } else {
            prop_assert!((watched.lift - motion::CLIP_LIFT * shown).abs() < 1e-4);
            prop_assert_eq!(watched.glow, shown);
        }
        prop_assert_eq!(watched.ring, shown);
        prop_assert_eq!(watched.dim, 0.0);
        prop_assert_eq!(other, WatchLook { dim: shown, ..WatchLook::default() });
    }
}

// ------------------------------------------------------------- track heights
