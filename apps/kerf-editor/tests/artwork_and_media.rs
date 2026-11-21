//! The Media band and the artwork behind it, through the public API: media
//! names, a tile's words, Roles and the Index, what the store keeps of
//! media events, the Cast dialog's file work, and the artwork pipeline
//! itself (FFmpeg's pictures and sound turned into posters, strips and
//! peaks) where FFmpeg is installed.

use kerf_editor::artwork::{
    self, fill_gaps, frame_index, parse_pam_stream, pixel_format_has_alpha, strip_frame_count,
    strip_height, waveform_columns, waveform_raise, Artwork, Bucket, PeakReducer, Peaks,
};
use kerf_editor::cast::{self, Entry};
use kerf_editor::media::{
    self, badge_runs, badge_text, index_rows, matches_search, program_time_of, role_change,
    role_choices, role_groups, role_name, status_text, step_choice, IndexRow,
};
use kerf_editor::model::{Clip, Source, Timeline};
use kerf_editor::names::{self, source_names, without_content_hash};
use kerf_editor::session::activity_of_command;
use kerf_editor::store::{making_status, Badge, EditorState, Making, MediaActivity};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

fn source(id: &str, path: &str) -> Source {
    Source {
        id: id.to_owned(),
        path: path.to_owned(),
        ..Source::default()
    }
}

// ------------------------------------------------------------------ names

#[test]
fn an_imports_hash_comes_off_its_name() {
    assert_eq!(without_content_hash("Beach Day-1a2b3c4d"), "Beach Day");
    assert_eq!(without_content_hash("Beach Day -1a2b3c4d"), "Beach Day");
    // Not eight lowercase hex digits after a hyphen: as it was.
    assert_eq!(
        without_content_hash("Beach Day-1A2B3C4D"),
        "Beach Day-1A2B3C4D"
    );
    assert_eq!(
        without_content_hash("Beach Day-1a2b3c4"),
        "Beach Day-1a2b3c4"
    );
    assert_eq!(
        without_content_hash("Beach Day_1a2b3c4d"),
        "Beach Day_1a2b3c4d"
    );
    // Nothing before the hash: kept whole.
    assert_eq!(without_content_hash("-1a2b3c4d"), "-1a2b3c4d");
    assert_eq!(without_content_hash(" -1a2b3c4d"), " -1a2b3c4d");
}

proptest! {
    /// Whatever was before the hash comes back, trailing spaces trimmed.
    #[test]
    fn the_hash_comes_off_any_stem(
        stem in "[A-Za-z0-9 ._()é]{0,12}[A-Za-z0-9é]",
        hash in "[0-9a-f]{8}",
        gap in " {0,2}",
    ) {
        let named = format!("{stem}{gap}-{hash}");
        prop_assert_eq!(without_content_hash(&named), stem.trim_end());
        // And taking it off twice changes nothing more.
        let once = without_content_hash(&named).to_owned();
        prop_assert_eq!(without_content_hash(&once), once.as_str());
    }
}

#[test]
fn media_read_alike_only_where_they_must() {
    let mut made = source("m4", "shots/m4.takes/001.mp4");
    made.steps = Some(2);
    let mut recipe = source("m5", "shots/boat.takes/003.mp4");
    recipe.recipe = Some("shots/boat@2.kerfrecipe".into());
    let sources = vec![
        source("m1", "media/Beach Day-1a2b3c4d.mov"),
        source("m2", "media/Beach Day-9f8e7d6c.mp4"),
        source("m3", "media/Harbour-00aa11bb.mov"),
        made,
        recipe,
        source("m6", "media/Same-1a2b3c4d.mov"),
        source("m7", "other/Same-1a2b3c4d.mov"),
        source("m8", ""),
    ];
    let names = source_names(&sources);
    // Two stems alike: the extension comes back.
    assert_eq!(names["m1"], "Beach Day.mov");
    assert_eq!(names["m2"], "Beach Day.mp4");
    assert_eq!(names["m3"], "Harbour");
    // A shot made by steps goes by its id, one from a recipe by the recipe.
    assert_eq!(names["m4"], "m4");
    assert_eq!(names["m5"], "boat");
    // Even the file names alike: the whole file name, hash and all.
    assert_eq!(names["m6"], "Same-1a2b3c4d.mov");
    assert_eq!(names["m7"], "Same-1a2b3c4d.mov");
    assert_eq!(names["m8"], names::UNTITLED);
}

proptest! {
    /// Every media has a name, and two media read alike only when their
    /// whole file names are alike too.
    #[test]
    fn names_tell_media_apart_unless_their_files_are_alike(
        stems in proptest::collection::vec("[ab]{1,2}", 1..8),
        extensions in proptest::collection::vec(prop_oneof!["mov", "mp4"], 8),
        hashes in proptest::collection::vec("[0-9a-f]{8}", 8),
    ) {
        let sources: Vec<Source> = stems
            .iter()
            .enumerate()
            .map(|(at, stem)| source(&format!("m{at}"), &format!("media/{stem}-{}.{}", hashes[at], extensions[at])))
            .collect();
        let names = source_names(&sources);
        prop_assert_eq!(names.len(), sources.len());
        for one in &sources {
            for other in &sources {
                if one.id != other.id && names[&one.id] == names[&other.id] {
                    prop_assert_eq!(one.file_name(), other.file_name());
                }
            }
        }
    }
}

fn snapshot_timeline() -> Value {
    json!({
        "fps": 24.0,
        "sources": [
            {"id": "a", "path": "media/Beach Day-1a2b3c4d.mov", "duration": 6.0, "width": 1280, "height": 720, "hasAudio": true},
            {"id": "t", "path": "media/tone.wav", "duration": 8.0, "hasAudio": true},
            {"id": "gone", "path": "", "duration": 2.0, "width": 640, "height": 360}
        ],
        "tracks": [
            {"id": "V1", "type": "video", "role": "video", "enabled": true, "clips": [
                {"id": "c1", "kind": "media", "sourceId": "a", "timelineStart": 0.0, "duration": 2.0, "in": 1.0, "out": 3.0, "speed": 1.0},
                {"id": "c2", "kind": "media", "sourceId": "a", "name": "Wide", "timelineStart": 2.0, "duration": 2.0, "in": 0.0, "out": 2.0},
                {"id": "c3", "kind": "media", "sourceId": "gone", "timelineStart": 4.0, "duration": 1.0},
                {"id": "c4", "kind": "title", "text": "Hello", "timelineStart": 5.0, "duration": 1.0}
            ]},
            {"id": "S1", "type": "subtitle", "role": "", "enabled": true, "clips": []},
            {"id": "A1", "type": "audio", "role": "dialogue", "enabled": true, "clips": [
                {"id": "s1", "kind": "media", "sourceId": "t", "timelineStart": 0.0, "duration": 4.0, "in": 0.0, "out": 4.0}
            ]},
            {"id": "A2", "type": "audio", "role": "Music", "roleAssigned": true, "enabled": false, "clips": [
                {"id": "s2", "kind": "media", "sourceId": "a", "timelineStart": 0.0, "duration": 4.0, "in": 0.0, "out": 4.0}
            ]}
        ]
    })
}

#[test]
fn a_clip_with_no_name_of_its_own_goes_by_its_medias() {
    let timeline = Timeline::from_value(&snapshot_timeline());
    let label = |id: &str| timeline.clip(id).unwrap().1.label().to_owned();
    assert_eq!(label("c1"), "Beach Day");
    // Its own name first; the document's name is left as it was.
    assert_eq!(label("c2"), "Wide");
    assert_eq!(timeline.clip("c1").unwrap().1.name, None);
    // Media with no file names none: the id stands.
    assert_eq!(label("c3"), "c3");
    assert_eq!(label("c4"), "Hello");
    assert_eq!(timeline.media_name("t"), "tone");
    assert_eq!(timeline.media_name("nope"), "nope");
}

#[test]
fn tracks_are_badged_and_media_say_where_they_are_used() {
    let timeline = Timeline::from_value(&snapshot_timeline());
    assert_eq!(timeline.track_badge("V1"), "V1");
    // Captions count among the picture tracks.
    assert_eq!(timeline.track_badge("S1"), "V2");
    assert_eq!(timeline.track_badge("A2"), "A2");
    let used = timeline.used_on();
    assert_eq!(used["a"], vec!["V1".to_owned(), "A2".to_owned()]);
    assert_eq!(used["t"], vec!["A1".to_owned()]);
}

#[test]
fn a_clip_shows_its_source_forwards_or_backwards() {
    let clip = Clip {
        source_in: Some(10.0),
        source_out: Some(14.0),
        duration: 2.0,
        speed: Some(2.0),
        ..Clip::default()
    };
    assert_eq!(clip.source_at(0.0), Some(10.0));
    assert_eq!(clip.source_at(1.0), Some(12.0));
    let reversed = Clip {
        reverse: true,
        ..clip.clone()
    };
    assert_eq!(reversed.source_at(0.0), Some(14.0));
    assert_eq!(reversed.source_at(2.0), Some(10.0));
    assert_eq!(Clip::default().source_at(1.0), None);
}

// ------------------------------------------------------------------ peaks

fn bucket() -> impl Strategy<Value = Bucket> {
    (-1.0f32..1.0, -1.0f32..1.0, 0.0f32..1.0).prop_map(|(a, b, rms)| Bucket {
        min: a.min(b),
        max: a.max(b),
        rms,
    })
}

proptest! {
    /// Each coarser level is the one below in fours, down to one bucket.
    #[test]
    fn coarser_levels_merge_four_of_the_one_below(buckets in proptest::collection::vec(bucket(), 1..300)) {
        let peaks = Peaks::from_buckets(buckets.clone());
        let levels = peaks.levels();
        prop_assert_eq!(&levels[0], &buckets);
        prop_assert_eq!(levels.last().unwrap().len(), 1);
        for pair in levels.windows(2) {
            prop_assert_eq!(pair[1].len(), pair[0].len().div_ceil(Peaks::MERGE));
            for (at, coarse) in pair[1].iter().enumerate() {
                let finer = &pair[0][at * Peaks::MERGE..((at + 1) * Peaks::MERGE).min(pair[0].len())];
                prop_assert_eq!(*coarse, Bucket::merge(finer));
            }
        }
        // The whole sound's extremes are the extremes of all of it.
        let whole = peaks.range(0.0, peaks.duration());
        let all = Bucket::merge(&buckets);
        prop_assert!((whole.min - all.min).abs() < 1e-6 && (whole.max - all.max).abs() < 1e-6);
    }

    /// A span reads the same either way round, never goes outside the
    /// sound's own extremes, and outside the sound is silence.
    #[test]
    fn a_span_reads_the_same_either_way_and_within_the_sound(
        buckets in proptest::collection::vec(bucket(), 1..200),
        from in 0.0f64..1.2,
        length in 0.0f64..1.2,
    ) {
        let peaks = Peaks::from_buckets(buckets.clone());
        let all = Bucket::merge(&buckets);
        let forward = peaks.range(from, from + length);
        let backward = peaks.range(from + length, from);
        prop_assert_eq!(forward, backward);
        prop_assert!(forward.min >= all.min.min(0.0) - 1e-6 && forward.max <= all.max.max(0.0) + 1e-6);
        let after = peaks.duration() + 1.0;
        prop_assert_eq!(peaks.range(after, after + 1.0), Bucket::default());
    }

    /// Raised for drawing, a level never leaves -1..1 and keeps its sign.
    #[test]
    fn a_drawn_level_stays_inside_the_clip(value in -1.0f32..1.0, raise in 0.0f64..8.0) {
        let level = artwork::waveform_level(value, raise);
        prop_assert!((-1.0..=1.0).contains(&level));
        prop_assert!(level == 0.0 || (level > 0.0) == (value > 0.0));
    }
}

#[test]
fn samples_reduce_to_five_millisecond_buckets() {
    let mut reducer = PeakReducer::new(16_000);
    // A second of a sine at half scale, fed in uneven chunks as bytes.
    let samples: Vec<f32> = (0..16_000).map(|n| 0.5 * (n as f32 * 0.05).sin()).collect();
    let bytes: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let mut rest = Vec::new();
    for chunk in bytes.chunks(1001) {
        reducer.add_bytes(&mut rest, chunk);
    }
    let peaks = reducer.finish();
    assert_eq!(peaks.levels()[0].len(), 200);
    assert!((peaks.duration() - 1.0).abs() < 1e-9);
    assert!((peaks.loudest() - 0.5).abs() < 0.01);
    // Raised so the loudest peak nearly fills the clip, at most 12 dB.
    assert!((peaks.display_gain() - 1.9).abs() < 0.05);
    assert!((waveform_raise(&peaks, 1.0) * peaks.loudest() - 0.95).abs() < 0.02);
    let quiet = Peaks::from_buckets(vec![Bucket {
        min: -0.01,
        max: 0.01,
        rms: 0.005,
    }]);
    assert_eq!(quiet.display_gain(), 4.0);
    let columns = waveform_columns(&peaks, 0.0, 1.0, 50, 1.0);
    assert_eq!(columns.len(), 50);
    assert!(columns
        .iter()
        .all(|column| column.low <= column.body_low && column.body_high <= column.high));
}

// --------------------------------------------------------------- pictures

fn pam(width: u32, height: u32, depth: u32, body: &[u8]) -> Vec<u8> {
    let tuple = match depth {
        1 => "GRAYSCALE",
        2 => "GRAYSCALE_ALPHA",
        3 => "RGB",
        _ => "RGB_ALPHA",
    };
    let mut bytes = format!(
        "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH {depth}\nMAXVAL 255\nTUPLTYPE {tuple}\nENDHDR\n"
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

proptest! {
    /// Pictures written back to back read back as they were, as RGBA.
    #[test]
    fn pam_pictures_read_back_as_written(
        pictures in proptest::collection::vec((1u32..6, 1u32..6, 1u32..5, any::<u8>()), 1..4),
    ) {
        let mut stream = Vec::new();
        let mut wanted = Vec::new();
        for (width, height, depth, seed) in &pictures {
            let body: Vec<u8> = (0..width * height * depth).map(|n| (n as u8).wrapping_mul(31).wrapping_add(*seed)).collect();
            stream.extend(pam(*width, *height, *depth, &body));
            let rgba: Vec<u8> = body
                .chunks(*depth as usize)
                .flat_map(|pixel| match depth {
                    1 => vec![pixel[0], pixel[0], pixel[0], 255],
                    2 => vec![pixel[0], pixel[0], pixel[0], pixel[1]],
                    3 => vec![pixel[0], pixel[1], pixel[2], 255],
                    _ => pixel.to_vec(),
                })
                .collect();
            wanted.push((*width, *height, rgba));
        }
        let read = parse_pam_stream(&stream);
        prop_assert_eq!(read.len(), wanted.len());
        for (picture, (width, height, rgba)) in read.iter().zip(&wanted) {
            prop_assert_eq!((picture.width, picture.height), (*width, *height));
            prop_assert_eq!(&picture.rgba, rgba);
        }
        // Cut short, what was whole still reads and the rest is left.
        let cut = &stream[..stream.len() - 1];
        prop_assert_eq!(parse_pam_stream(cut).len(), wanted.len() - 1);
    }

    /// Garbage never panics and reads as no pictures, or as those whole.
    #[test]
    fn garbage_reads_as_nothing(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
        let _ = parse_pam_stream(&bytes);
    }
}

#[test]
fn a_strip_is_spread_along_its_source() {
    assert_eq!(strip_frame_count(0.0), artwork::STRIP_FRAMES);
    assert_eq!(strip_frame_count(f64::NAN), artwork::STRIP_FRAMES);
    assert_eq!(strip_frame_count(2.0), 16);
    assert_eq!(strip_frame_count(60.0), 64);
    assert_eq!(frame_index(0, 0.5, 1.0), None);
    assert_eq!(frame_index(10, 0.5, 1.0), Some(2));
    assert_eq!(frame_index(10, 0.5, 99.0), Some(9));
    assert_eq!(frame_index(10, 0.5, -3.0), Some(0));
    assert_eq!(strip_height(30.0), 48);
    assert_eq!(strip_height(88.0), 96);
    assert_eq!(strip_height(150.0), 192);
    assert_eq!(
        fill_gaps(vec![None, Some(1), None, Some(3), None]),
        Some(vec![1, 1, 1, 3, 3])
    );
    assert_eq!(fill_gaps::<u8>(vec![None, None]), None);
}

#[test]
fn alpha_is_read_from_the_pixel_format() {
    for format in [
        "yuva420p",
        "rgba",
        "argb",
        "bgra",
        "gbrap12le",
        "ya8",
        "rgba64le",
    ] {
        assert!(pixel_format_has_alpha(format), "{format}");
    }
    for format in ["", "yuv420p", "rgb24", "gbrp", "nv12", "pal8"] {
        assert!(!pixel_format_has_alpha(format), "{format}");
    }
}

#[test]
fn a_picture_is_fitted_at_its_own_shape() {
    use gpui::{point, px, size, Bounds};
    let tile = Bounds {
        origin: point(px(10.0), px(20.0)),
        size: size(px(112.0), px(63.0)),
    };
    // A square inside a wide tile: bars either side, centred.
    let inside = artwork::fitted(tile, (100, 100), false);
    assert_eq!(inside.size.height, px(63.0));
    assert_eq!(inside.size.width, px(63.0));
    assert_eq!(inside.origin.x, px(10.0 + (112.0 - 63.0) / 2.0));
    // Covering: it fills the tile, its edges cut off.
    let cover = artwork::fitted(tile, (100, 100), true);
    assert_eq!(cover.size.width, px(112.0));
    assert!(cover.size.height >= px(63.0));
}

// ------------------------------------------------------------------ tiles

#[test]
fn a_badge_counts_frames_at_the_sequences_rate() {
    assert_eq!(badge_text(9.0 + 20.0 / 24.0, 24.0), "9s20f");
    assert_eq!(badge_text(0.0, 24.0), "0s00f");
    assert_eq!(badge_text(65.0, 24.0), "1m05s");
    assert_eq!(badge_text(3720.0, 25.0), "1h02m");
    // No rate given: 24.
    assert_eq!(badge_text(1.5, 0.0), "1s12f");
    assert_eq!(badge_text(f64::NAN, 24.0), "0s00f");
}

#[test]
fn a_tiles_second_line_says_what_it_is_doing_or_is() {
    let now = Instant::now();
    let video = Source {
        id: "m1".into(),
        path: "media/a.mov".into(),
        duration: 4.0,
        width: Some(1920),
        ..Source::default()
    };
    let on = vec!["V1".to_owned(), "V2".to_owned()];
    let say =
        |source: &Source,
         activity: MediaActivity,
         badge: Option<Badge>,
         wants: Option<&str>,
         used: &[String]| { status_text(source, &activity, badge, wants, used, now) };
    assert_eq!(
        say(&video, MediaActivity::Ready, None, None, &on),
        "Video · on V1 V2"
    );
    let still = Source {
        duration: 0.0,
        ..video.clone()
    };
    assert_eq!(say(&still, MediaActivity::Ready, None, None, &[]), "Still");
    let sound = Source {
        width: None,
        has_audio: true,
        ..video.clone()
    };
    assert_eq!(
        say(
            &sound,
            MediaActivity::Ready,
            Some(Badge::External),
            None,
            &[]
        ),
        "Sound · outside the project"
    );
    assert_eq!(
        say(
            &video,
            MediaActivity::Ready,
            Some(Badge::NotDownloaded),
            None,
            &on
        ),
        "Not downloaded · on V1 V2"
    );
    assert_eq!(
        say(
            &video,
            MediaActivity::Offline,
            Some(Badge::Offline),
            None,
            &on
        ),
        "Offline · file missing"
    );
    let making = MediaActivity::Making {
        take: 2,
        since: now - Duration::from_secs(41),
    };
    assert_eq!(say(&video, making, None, None, &on), "Making take 2 · 0:41");
    assert_eq!(
        say(
            &video,
            MediaActivity::Queued { ahead: Some(1) },
            None,
            None,
            &on
        ),
        "Queued · 1 ahead"
    );
    assert_eq!(
        say(
            &video,
            MediaActivity::Queued { ahead: None },
            None,
            None,
            &on
        ),
        "Queued for making"
    );
    let mut shot = video.clone();
    shot.steps = Some(1);
    shot.takes = vec![
        kerf_editor::model::Take {
            number: 1,
            path: "a".into(),
            chosen: true,
        },
        kerf_editor::model::Take {
            number: 2,
            path: "b".into(),
            chosen: false,
        },
    ];
    assert_eq!(
        say(&shot, MediaActivity::Ready, None, None, &["V1".to_owned()]),
        "Made · shot · take 1 of 2 · on V1"
    );
    assert_eq!(
        say(
            &shot,
            MediaActivity::NotMade,
            Some(Badge::Offline),
            None,
            &[]
        ),
        "Not made yet"
    );
    assert_eq!(
        say(
            &shot,
            MediaActivity::Ready,
            None,
            Some("cast Maya changed"),
            &[]
        ),
        "Wants making · cast Maya changed"
    );
    shot.takes.clear();
    assert_eq!(
        say(&shot, MediaActivity::Ready, None, None, &[]),
        "Made · shot · take 1 of 1"
    );
}

#[test]
fn a_tiles_tooltip_says_its_length_file_and_what_to_do() {
    let video = Source {
        id: "m1".into(),
        path: "media/a.mov".into(),
        duration: 2.5,
        width: Some(1920),
        ..Source::default()
    };
    let tip = media::tile_tip("a", &video, 24.0, "Video", Some(Badge::External));
    let lines: Vec<&str> = tip.lines().collect();
    assert_eq!(lines[0], "a · 00:00:02:12");
    assert_eq!(lines[1], "Video");
    assert_eq!(lines[2], "media/a.mov");
    assert_eq!(lines[3], "Double-click to insert at the playhead");
    assert!(lines[4].contains("Consolidate Media"));
}

#[test]
fn search_keeps_names_holding_what_is_typed() {
    assert!(matches_search("Beach Day.mov", "beach"));
    assert!(matches_search("Beach Day.mov", "  "));
    assert!(!matches_search("Harbour", "beach"));
    assert!(matches_search("ÉTÉ", "été"));
}

proptest! {
    /// The arrow keys land on a media shown, held at either end.
    #[test]
    fn the_arrow_keys_stay_among_the_media_shown(
        count in 0usize..8,
        current in proptest::option::of(0usize..10),
        step in -3i32..4,
    ) {
        let ids: Vec<String> = (0..count).map(|n| format!("m{n}")).collect();
        let current = current.map(|n| format!("m{n}"));
        let next = step_choice(&ids, current.as_deref(), step);
        match next {
            None => prop_assert!(ids.is_empty()),
            Some(next) => {
                let at = ids.iter().position(|id| *id == next).unwrap();
                if let Some(from) = current.as_ref().and_then(|current| ids.iter().position(|id| id == current)) {
                    prop_assert_eq!(at as i64, (from as i64 + i64::from(step)).clamp(0, ids.len() as i64 - 1));
                }
            }
        }
    }
}

// ------------------------------------------------------------------ roles

#[test]
fn roles_group_the_programs_tracks() {
    let timeline = Timeline::from_value(&snapshot_timeline());
    let groups = role_groups(&timeline.tracks);
    let shown: Vec<(&str, &str, Vec<&str>, bool)> = groups
        .iter()
        .map(|group| {
            (
                group.key.as_str(),
                group.name.as_str(),
                group.tracks.iter().map(String::as_str).collect(),
                group.enabled,
            )
        })
        .collect();
    assert_eq!(
        shown,
        vec![
            ("video", "Video", vec!["V1"], true),
            ("captions", "Captions", vec!["S1"], true),
            ("dialogue", "Dialogue", vec!["A1"], true),
            ("music", "Music", vec!["A2"], false),
        ]
    );
    assert_eq!(role_name("", "audio"), "Audio");
    assert_eq!(role_name("", ""), "Other");
    assert_eq!(
        media::enable_ops(&["A1".into(), "A2".into()], false),
        vec![
            json!({"op": "set-track", "track": "A1", "enabled": false}),
            json!({"op": "set-track", "track": "A2", "enabled": false}),
        ]
    );
}

#[test]
fn badges_in_a_row_read_as_a_run() {
    let badges = |list: &[&str]| {
        list.iter()
            .map(|badge| (*badge).to_owned())
            .collect::<Vec<_>>()
    };
    // Qt's badgeRuns: a run's last badge is said in full, "V1–V3".
    assert_eq!(badge_runs(&badges(&["V1", "V2", "V3", "A1"])), "V1–V3 A1");
    assert_eq!(
        badge_runs(&badges(&["V1", "V2", "A1", "A2", "A3", "A5"])),
        "V1 V2 A1–A3 A5"
    );
    let twelve: Vec<String> = (1..=12).map(|n| format!("V{n}")).collect();
    assert_eq!(badge_runs(&twelve), "V1–V12");
    assert_eq!(badge_runs(&badges(&[])), "");
}

proptest! {
    /// Every badge is said, in order: one by one, or as the first and the
    /// last of a run of three or more.
    #[test]
    fn a_run_names_its_first_and_last_badge(
        lengths in proptest::collection::vec((0usize..2, 1usize..6), 0..5),
    ) {
        let mut badges = Vec::new();
        let mut said = Vec::new();
        let mut next = [1u32, 1];
        for (kind, length) in lengths {
            let letter = ["V", "A"][kind];
            // A gap of one after each run keeps runs apart.
            let first = next[kind];
            let run: Vec<String> = (first..first + length as u32)
                .map(|number| format!("{letter}{number}"))
                .collect();
            next[kind] = first + length as u32 + 1;
            if run.len() >= 3 {
                said.push(format!("{}–{}", run[0], run[run.len() - 1]));
            } else {
                said.extend(run.iter().cloned());
            }
            badges.extend(run);
        }
        prop_assert_eq!(badge_runs(&badges), said.join(" "));
    }
}

#[test]
fn edit_roles_sends_only_what_changed() {
    let timeline = Timeline::from_value(&snapshot_timeline());
    let choices = role_choices(&timeline.tracks);
    assert_eq!(
        &choices[..5],
        ["Video", "Titles", "Dialogue", "Music", "Effects"]
    );
    // A listed role by its engine name; the default as nothing.
    assert_eq!(role_change("Music", &choices, ""), Some("music".into()));
    assert_eq!(role_change("music", &choices, "music"), None);
    assert_eq!(role_change("", &choices, "music"), Some(String::new()));
    assert_eq!(
        role_change("Dialogue (default)", &choices, ""),
        Some("dialogue".into())
    );
    // A role typed is sent as typed.
    assert_eq!(
        role_change(" Room Tone ", &choices, ""),
        Some("Room Tone".into())
    );
    assert_eq!(
        media::role_ops(&[("A2".into(), String::new())]),
        vec![json!({"op": "set-track", "track": "A2", "role": ""})]
    );
}

// ------------------------------------------------------------------ index

#[test]
fn the_index_lists_shots_silences_and_loud_moments() {
    let index = json!({
        "media": "m1", "duration": 10.0,
        "cuts": [6.0, 2.5], "ranges": [],
        "silences": [[1.0, 1.5]],
        "loudest": [4.0, 8.25]
    });
    let rows = index_rows(&index, 10.0, 24.0);
    assert_eq!(rows[0], IndexRow::Header("3 shots".into()));
    assert_eq!(
        rows[1],
        IndexRow::Row {
            name: "Shot 1".into(),
            meta: "00:00:00:00–00:00:02:12".into(),
            at: 0.0
        }
    );
    assert_eq!(
        rows[3],
        IndexRow::Row {
            name: "Shot 3".into(),
            meta: "00:00:06:00–00:00:10:00".into(),
            at: 6.0
        }
    );
    assert_eq!(rows[4], IndexRow::Header("1 silence".into()));
    assert_eq!(rows[6], IndexRow::Header("Loudest moments".into()));
    assert_eq!(rows.len(), 9);
    // The engine's ranges come before the spans between cuts.
    let ranges = json!({"ranges": [{"start": 0.0, "end": 3.0}], "cuts": [1.0]});
    assert_eq!(
        index_rows(&ranges, 3.0, 24.0)[0],
        IndexRow::Header("1 shot".into())
    );
    assert!(index_rows(&json!({}), 3.0, 24.0).is_empty());
}

#[test]
fn an_index_row_finds_where_the_program_shows_it() {
    let timeline = Timeline::from_value(&snapshot_timeline());
    // c1 shows a 1..3 s at 0..2 s of the program.
    assert_eq!(program_time_of(&timeline, "a", 1.5), Some(0.5));
    assert_eq!(program_time_of(&timeline, "a", 9.0), None);
    assert_eq!(program_time_of(&timeline, "nope", 1.0), None);
}

// ------------------------------------------------------------------ store

fn state_with_media() -> EditorState {
    let mut state = EditorState::default();
    let mut timeline = snapshot_timeline();
    timeline["sources"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "shot", "path": "shots/shot.mp4", "duration": 2.0, "width": 1920, "height": 1080, "steps": 1}));
    state.apply(&json!({"type": "state", "snapshot": {"timeline": timeline}}));
    state
}

#[test]
fn media_status_badges_with_missing_first() {
    let mut state = state_with_media();
    state.apply(&json!({"type": "media_status", "missing": ["a", "shot"], "pointers": ["a", "t"], "external": ["t", "gone"]}));
    let status = &state.media_status;
    assert_eq!(status.badge("a"), Some(Badge::Offline));
    assert_eq!(status.badge("t"), Some(Badge::NotDownloaded));
    assert_eq!(status.badge("gone"), Some(Badge::External));
    assert_eq!(status.badge("x"), None);
    // A missing shot made from steps is not made yet; other media is offline.
    assert_eq!(state.media_activity("shot"), MediaActivity::NotMade);
    assert_eq!(state.media_activity("a"), MediaActivity::Offline);
    assert_eq!(state.media_activity("t"), MediaActivity::Ready);
}

#[test]
fn a_making_waiting_its_turn_is_queued_until_it_runs() {
    let mut state = state_with_media();
    state.apply(&json!({"type": "media_making", "media": "shot", "take": 1, "line": "asked the server for 48 frames: job 7"}));
    assert_eq!(
        state.media_activity("shot"),
        MediaActivity::Queued { ahead: None }
    );
    state.apply(&json!({"type": "media_making", "media": "shot", "take": 1, "line": "waiting, 2 ahead, 4 s"}));
    assert_eq!(
        state.media_activity("shot"),
        MediaActivity::Queued { ahead: Some(2) }
    );
    state.apply(
        &json!({"type": "media_making", "media": "shot", "take": 1, "line": "running, 2 s"}),
    );
    assert!(matches!(
        state.media_activity("shot"),
        MediaActivity::Making { take: 1, .. }
    ));
    let making = state.makings["shot"].clone();
    assert_eq!(making_status("shot", &making), "Making shot: running, 2 s");
    let starting = Making {
        line: String::new(),
        ..making
    };
    assert_eq!(
        making_status("shot", &starting),
        "Making take 1 of shot from its steps…"
    );
    state.apply(&json!({"type": "media_made", "media": "shot", "take": 1}));
    assert_eq!(
        state.notices.last().unwrap().text,
        "Made take 1 of shot from its steps"
    );
    assert!(!state.notices.last().unwrap().error);
    state.apply(&json!({"type": "media_making", "media": "a", "take": 1, "line": ""}));
    state.apply(&json!({"type": "media_make_cancelled", "media": "a"}));
    assert_eq!(
        state.notices.last().unwrap().text,
        "Stopped making a; it is as it was"
    );
    assert!(state.makings.is_empty());
}

#[test]
fn media_info_and_indexes_are_kept_and_go_with_a_relink() {
    let mut state = state_with_media();
    state.apply(&json!({"type": "media_info", "media": {"a": {"pixFmt": "yuva420p"}}}));
    state.apply(&json!({"type": "media_info", "media": {"t": {"codec": "pcm"}}}));
    assert_eq!(state.media_info["a"]["pixFmt"], "yuva420p");
    assert_eq!(state.media_info.len(), 2);
    state.index_asked.insert("a".into());
    state.apply(&json!({"type": "media_index", "media": "a", "cuts": [1.0]}));
    assert!(state.media_indexes.contains_key("a") && state.index_asked.is_empty());
    // A refusal is said on the Index page, not over the window.
    let notices = state.notices.len();
    state.apply(
        &json!({"type": "error", "command": "media_index", "media": "t", "message": "No sound"}),
    );
    assert_eq!(state.media_index_errors["t"], "No sound");
    assert_eq!(state.notices.len(), notices);
    // The same file again keeps them; another file lets them go.
    let mut timeline = snapshot_timeline();
    state.apply(&json!({"type": "state", "snapshot": {"timeline": timeline.clone()}}));
    assert!(state.media_indexes.contains_key("a"));
    timeline["sources"][0]["path"] = json!("media/other.mov");
    state.apply(&json!({"type": "state", "snapshot": {"timeline": timeline}}));
    assert!(!state.media_indexes.contains_key("a"));
    assert!(!state.media_info.contains_key("a"));
    assert!(state.media_index_errors.contains_key("t"));
}

#[test]
fn a_cast_says_what_wants_making_and_saving_says_so() {
    let mut state = state_with_media();
    let cast = json!({"type": "cast", "saved": false, "entries": [],
                      "toMake": [{"media": "shot", "why": "cast Maya changed"}]});
    state.apply(&cast);
    assert_eq!(state.wants_making["shot"], "cast Maya changed");
    assert!(state.notices.is_empty());
    let saved = json!({"type": "cast", "saved": true, "entries": [],
                       "toMake": [{"media": "shot", "why": "x"}, {"media": "m9", "why": "y"}]});
    state.apply(&saved);
    assert_eq!(
        state.notices.last().unwrap().text,
        "Saved the cast; shot, m9 want making again"
    );
    state.apply(&json!({"type": "media_made", "media": "shot", "take": 2}));
    assert!(!state.wants_making.contains_key("shot"));
    let none = json!({"type": "cast", "saved": true, "toMake": []});
    assert_eq!(
        kerf_editor::store::cast_saved(&none),
        "Saved the cast; no shot wants making for it"
    );
    // What is wrong with a cast is said in its dialog.
    let notices = state.notices.len();
    state.apply(&json!({"type": "error", "command": "cast", "message": "line 2: no name"}));
    assert_eq!(state.notices.len(), notices);
}

#[test]
fn a_new_connection_asks_again() {
    let mut state = state_with_media();
    state.info_asked.insert("a".into());
    state.index_asked.insert("a".into());
    state.apply(&json!({"type": "connection", "status": "connected"}));
    assert!(state.info_asked.is_empty() && state.index_asked.is_empty());
}

// --------------------------------------------------------------- activity

#[test]
fn makings_takes_and_carries_leave_activity_lines() {
    let mut timeline = Timeline::from_value(&snapshot_timeline());
    let mut shot = source("m4", "shots/m4.takes/002.mp4");
    shot.steps = Some(2);
    shot.takes = vec![kerf_editor::model::Take {
        number: 1,
        path: "shots/m4.takes/001.mp4".into(),
        chosen: false,
    }];
    timeline.sources.push(shot);
    let line = |command: Value| activity_of_command(&command, &timeline).map(|(_, text)| text);
    assert_eq!(
        line(json!({"type": "media_make", "media": "m4", "confirmed": true})).as_deref(),
        Some("Making m4 from its steps in the document")
    );
    assert_eq!(
        line(json!({"type": "media_make", "media": "m4", "confirmed": true, "rolls": 3}))
            .as_deref(),
        Some("Making 3 takes of m4 from its steps in the document")
    );
    // Asking for the plan is not a making.
    assert_eq!(line(json!({"type": "media_make", "media": "m4"})), None);
    assert_eq!(
        line(json!({"type": "media_carry", "media": "m5", "confirmed": true})).as_deref(),
        Some("Carrying the steps of m5 into the document")
    );
    assert_eq!(
        line(json!({"type": "media_replace", "media": "m4", "path": "shots/m4.takes/001.mp4"}))
            .as_deref(),
        Some("Using take 1 of m4")
    );
    assert_eq!(
        line(json!({"type": "media_replace", "media": "a", "path": "/home/me/b.mov"})).as_deref(),
        Some("Replacing a with b.mov")
    );
    assert_eq!(line(json!({"type": "seek", "position": 1.0})), None);
}

// ------------------------------------------------------------------- cast

#[test]
fn the_cast_is_read_and_written_as_the_file_reads() {
    let event = json!({"type": "cast", "entries": [
        {"kind": "cast", "name": "Maya", "words": "a woman", "refs": ["refs/maya.png"]},
        {"kind": "style", "name": "Dusk", "words": "", "refs": []}
    ], "toMake": [{"media": "m3", "why": "cast Maya changed"}]});
    let entries = cast::entries_of(&event);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].refs, vec!["refs/maya.png".to_owned()]);
    assert_eq!(
        cast::wants_line(&cast::to_make_of(&event)),
        "Wants making now: m3 (cast Maya changed)"
    );
    assert_eq!(
        cast::wants_line(&[]),
        "No shot wants making as the cast stands."
    );
    let typed = vec![Entry {
        kind: "cast".into(),
        name: "  Maya ".into(),
        words: " a  woman\nin   red ".into(),
        refs: cast::split_pictures(" refs/a.png, ,refs/b.png ,"),
    }];
    assert_eq!(
        cast::cast_set(&typed),
        json!({"type": "cast_set", "entries": [
            {"kind": "cast", "name": "Maya", "words": "a woman in red", "refs": ["refs/a.png", "refs/b.png"]}
        ]})
    );
}

#[test]
fn a_cast_that_does_not_read_cannot_be_saved_over() {
    use cast::{heard, Heard, Stage};
    let unreadable = json!({"type": "error", "command": "cast",
        "message": "cast.kerfcast:4: `csat` is not `cast` or `style`"});
    // Asked for and not read: nothing to show, and nothing to save over the file.
    assert_eq!(
        heard(Stage::Reading, &unreadable),
        Heard::Failed {
            message: "cast.kerfcast:4: `csat` is not `cast` or `style`".into(),
            stage: Stage::Unreadable,
        }
    );
    assert!(!Stage::Unreadable.can_save());
    assert!(!Stage::Reading.can_save() && !Stage::Saving.can_save());
    // A save refused leaves the rows as typed, to be mended and saved again.
    let refused = json!({"type": "error", "command": "cast", "message": "cannot save the cast: two named Ada"});
    assert_eq!(
        heard(Stage::Saving, &refused),
        Heard::Failed {
            message: "cannot save the cast: two named Ada".into(),
            stage: Stage::Editing,
        }
    );
    assert!(Stage::Editing.can_save());
    // Read, the rows come in; saved, the window goes back.
    let read = json!({"type": "cast", "entries": [{"kind": "cast", "name": "mara", "words": "", "refs": []}],
        "toMake": [], "saved": false});
    assert!(matches!(heard(Stage::Reading, &read), Heard::Read(entries) if entries.len() == 1));
    let saved = json!({"type": "cast", "entries": [], "toMake": [], "saved": true});
    assert_eq!(heard(Stage::Saving, &saved), Heard::Saved);
    assert_eq!(heard(Stage::Editing, &saved), Heard::ToMake);
    // Another command's error is not the dialog's.
    let other = json!({"type": "error", "command": "media_make", "message": "no"});
    assert_eq!(heard(Stage::Reading, &other), Heard::Nothing);
}

/// The dialog's stage after each event, as `CastView::heard` moves it.
fn cast_stage_after(stage: cast::Stage, event: &Value) -> cast::Stage {
    match cast::heard(stage, event) {
        cast::Heard::Read(_) => cast::Stage::Editing,
        cast::Heard::Failed { stage, .. } => stage,
        _ => stage,
    }
}

proptest! {
    /// Save is pressed only on a cast that was read: whatever kerfd says,
    /// in whatever order, the dialog can save only once a `cast` it asked
    /// for came in while it was reading.
    #[test]
    fn save_needs_the_cast_read_first(events in proptest::collection::vec(0usize..5, 0..12)) {
        let said = [
            json!({"type": "error", "command": "cast", "message": "does not read"}),
            json!({"type": "error", "command": "media_make", "message": "no"}),
            json!({"type": "cast", "entries": [], "toMake": [], "saved": true}),
            json!({"type": "snapshot"}),
            json!({"type": "cast", "entries": [], "toMake": [], "saved": false}),
        ];
        let mut stage = cast::Stage::Reading;
        let mut read = false;
        for event in events {
            // A `cast` holds the file's entries, saved by another or not.
            if stage == cast::Stage::Reading && (event == 2 || event == 4) {
                read = true;
            }
            stage = cast_stage_after(stage, &said[event]);
            if stage.can_save() {
                prop_assert!(read);
            }
        }
    }
}

#[test]
fn a_picture_from_outside_is_copied_into_refs_under_a_free_name() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let face = outside.path().join("face.png");
    std::fs::write(&face, b"one").unwrap();
    assert_eq!(
        cast::cast_picture_path(project.path(), &face).unwrap(),
        "refs/face.png"
    );
    std::fs::write(&face, b"two").unwrap();
    assert_eq!(
        cast::cast_picture_path(project.path(), &face).unwrap(),
        "refs/face-2.png"
    );
    assert_eq!(
        std::fs::read(project.path().join("refs/face-2.png")).unwrap(),
        b"two"
    );
    // One of the project's own is named as it is.
    let inside = project.path().join("shots").join("still.png");
    std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
    std::fs::write(&inside, b"x").unwrap();
    assert_eq!(
        cast::cast_picture_path(project.path(), &inside).unwrap(),
        "shots/still.png"
    );
    // One that is not there cannot be copied.
    assert!(cast::cast_picture_path(project.path(), Path::new("/nowhere/x.png")).is_err());
}

// ---------------------------------------------------------------- artwork

fn ffmpeg() -> Option<String> {
    let binary = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    std::process::Command::new(&binary)
        .arg("-version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|_| binary)
}

/// Waits for `ready` for up to 60 s, as the workers make the artwork.
fn wait_for(artwork: &Artwork, ready: impl Fn(&Artwork) -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if ready(artwork) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn media_gets_a_poster_a_strip_and_peaks_off_the_window() {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no FFmpeg here: the artwork pipeline is not exercised");
        return;
    };
    let project = tempfile::tempdir().unwrap();
    let made = std::process::Command::new(&ffmpeg)
        .args([
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x180:r=24:d=2",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=2",
            "-shortest",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(project.path().join("clip.mp4"))
        .status()
        .unwrap();
    assert!(made.success());
    std::fs::write(project.path().join("broken.mov"), b"not a movie at all").unwrap();
    let clip = Source {
        id: "m1".into(),
        path: "clip.mp4".into(),
        duration: 2.0,
        has_audio: true,
        width: Some(320),
        height: Some(180),
        ..Source::default()
    };
    let broken = Source {
        id: "m2".into(),
        path: "broken.mov".into(),
        duration: 2.0,
        width: Some(320),
        ..Source::default()
    };
    let missing = Source {
        id: "m3".into(),
        path: "gone.mov".into(),
        width: Some(320),
        has_audio: true,
        ..Source::default()
    };
    let artwork = Artwork::new(Some(project.path().to_path_buf()));
    artwork.sync(&[clip.clone(), broken.clone(), missing.clone()]);
    assert!(wait_for(&artwork, |artwork| artwork.poster("m1").is_some()
        && artwork.peaks("m1").is_some()));
    let poster = artwork.poster("m1").unwrap();
    // At its own shape, covering the poster's size.
    let (width, height) = artwork::image_size(&poster);
    assert_eq!(height, artwork::POSTER_SIZE.1);
    assert!((width as f32 / height as f32 - 16.0 / 9.0).abs() < 0.02);
    let peaks = artwork.peaks("m1").unwrap();
    assert!((peaks.duration() - 2.0).abs() < 0.05);
    assert!(peaks.loudest() > 0.05);
    assert!(artwork.take_changed());
    // A strip is asked for when a view wants one; the poster stands in.
    assert!(artwork.frame_at("m1", 1.0, 40.0).is_some());
    assert!(wait_for(&artwork, |artwork| !artwork.busy()));
    let frame = artwork.frame_at("m1", 1.0, 40.0).unwrap();
    assert_eq!(artwork::image_size(&frame).1, 48);
    // What does not decode, or is not there, has none, and asks no more.
    assert!(artwork.poster("m2").is_none() && artwork.peaks("m2").is_none());
    assert!(artwork.poster("m3").is_none() && artwork.frame_at("m3", 0.0, 96.0).is_none());
    assert!(!artwork.busy());
    // Media taken out lets go of its pictures.
    artwork.sync(&[broken]);
    assert!(artwork.poster("m1").is_none());
    assert!(!artwork.take_dropped().is_empty());
}
