//! Media: the media bin, waveforms and pictures of media, importing, the inspector, and Finish's checks and deliveries.

use crate::support::{command, snapshot_timeline};
use gpui::{
    div, point, px, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    Modifiers, ParentElement, Render, Styled, TestAppContext, VisualTestContext, Window,
};
use kerf_bridge::event::RenderQuality;
use kerf_editor::artwork::{
    fill_gaps, frame_index, parse_pam_stream, pixel_format_has_alpha, strip_frame_count,
    strip_height, waveform_columns, waveform_raise, Artwork, Bucket, PeakReducer, Peaks,
};
use kerf_editor::delivery::{
    export_request, master_goes, master_makes, master_request, request, Quality, DELIVERIES,
};
use kerf_editor::export::{export_into, export_tip};
use kerf_editor::finish::{
    deliver_said, other_branches, preflight_hidden, preflight_rows, publish_command, publish_name,
    pull_request_command, pull_request_said, pull_request_title, sync_lines, sync_said, PublishWay,
    SyncFailure,
};
use kerf_editor::inspector::{
    choice_is_compact, choice_menu_opens_up, choice_menu_shift, choice_menu_stays, choice_rows,
    choice_sends, format_beside, format_index, scroll_to_show, InspectorView,
};
use kerf_editor::media::{
    badge_runs, badge_text, index_rows, matches_search, program_time_of, role_change, role_choices,
    role_groups, role_name, status_text, step_choice, IndexRow,
};
use kerf_editor::model::{Clip, Source, Timeline, TimelineShow};
use kerf_editor::prefs::{MasterOutput, Prefs};
use kerf_editor::session::Session;
use kerf_editor::store::{Badge, MediaActivity};
use kerf_editor::theme::{layout, space};
use kerf_editor::ui::menu_letter;
use kerf_editor::{artwork, input, media};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    shot.steps = 1;
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
    // A run's last badge is said in full, "V1–V3".
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

// ---------------------------------------------------------------- a failed sync

#[test]
fn a_failed_sync_stands_until_a_later_one_starts_or_succeeds() {
    let mut sync = SyncFailure::default();
    assert!(sync.failed("could not reach origin", false));
    assert_eq!(sync.standing(), Some("could not reach origin"));
    assert_eq!(sync.last(), Some("could not reach origin"));
    // A later sync starts: it stands no longer, but the chip still knows why
    // the last one failed until one succeeds.
    sync.started();
    assert_eq!(sync.standing(), None);
    assert_eq!(sync.last(), Some("could not reach origin"));
    // It fails the same way again: Live Sync says it only once, but it
    // stands again.
    assert!(!sync.failed("could not reach origin", false));
    assert_eq!(sync.standing(), Some("could not reach origin"));
    // A person asking hears it each time.
    assert!(sync.failed("could not reach origin", true));
    // Another reason is said.
    assert!(sync.failed("remote refused the push", false));
    sync.done();
    assert_eq!(sync.standing(), None);
    assert_eq!(sync.last(), None);
    // After a success the same reason is new again.
    assert!(sync.failed("could not reach origin", false));
}

#[derive(Clone, Debug)]
enum Step {
    Started,
    Failed(u8, bool),
    Done,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        Just(Step::Started),
        (0u8..3, proptest::bool::ANY).prop_map(|(why, asked)| Step::Failed(why, asked)),
        Just(Step::Done),
    ]
}

proptest! {
    #[test]
    fn a_sync_failure_follows_its_rules(steps in proptest::collection::vec(step(), 0..40)) {
        let mut sync = SyncFailure::default();
        // The model: why the last failed (until done), whether it stands.
        let mut last: Option<String> = None;
        for step in steps {
            let standing = match step {
                Step::Started => {
                    sync.started();
                    false
                }
                Step::Failed(why, asked) => {
                    let why = format!("reason {why}");
                    let said = sync.failed(&why, asked);
                    prop_assert_eq!(said, asked || last.as_deref() != Some(why.as_str()));
                    last = Some(why);
                    true
                }
                Step::Done => {
                    sync.done();
                    last = None;
                    false
                }
            };
            prop_assert_eq!(sync.last(), last.as_deref());
            prop_assert_eq!(sync.standing(), last.as_deref().filter(|_| standing));
            // What stands is always why the last one failed.
            if let Some(why) = sync.standing() {
                prop_assert_eq!(Some(why), sync.last());
            }
        }
    }
}

// ------------------------------------------------------------ branches

proptest! {
    /// The branches to switch to are the others, each once, in git's order.
    #[test]
    fn the_branches_to_switch_to_are_the_others_once_each(
        branches in proptest::collection::vec("[a-c]{0,2}", 0..8),
        current in proptest::option::of("[a-c]{0,2}"),
    ) {
        let others = other_branches(current.as_deref(), &branches);
        for (at, branch) in others.iter().enumerate() {
            prop_assert!(!branch.is_empty());
            prop_assert!(Some(branch.as_str()) != current.as_deref());
            prop_assert!(!others[..at].contains(branch));
            prop_assert!(branches.contains(branch));
        }
        for branch in &branches {
            if !branch.is_empty() && Some(branch.as_str()) != current.as_deref() {
                prop_assert!(others.contains(branch));
            }
        }
        // In the order git lists them.
        let firsts: Vec<&String> = branches
            .iter()
            .filter(|branch| others.contains(branch))
            .fold(Vec::new(), |mut seen, branch| {
                if !seen.contains(&branch) {
                    seen.push(branch);
                }
                seen
            });
        prop_assert_eq!(firsts, others.iter().collect::<Vec<_>>());
    }
}

fn owned(options: &[&str]) -> Vec<String> {
    options.iter().map(|option| (*option).to_owned()).collect()
}

/// What a choice's menu keeps from its button and from the window's edge.
const MARGIN: f32 = space::S4 + space::S8;

#[test]
fn a_few_short_options_are_segments_and_the_rest_open_a_menu() {
    assert!(choice_is_compact(&owned(&["left", "center", "right"])));
    assert!(choice_is_compact(&owned(&["in", "out"])));
    // A fourth option, or one longer than eight letters, opens a menu.
    assert!(!choice_is_compact(&owned(&[
        "normal", "add", "screen", "multiply"
    ])));
    assert!(!choice_is_compact(&owned(&["linear", "character"])));
}

#[test]
fn the_menu_says_each_option_in_words_and_ticks_the_one_chosen() {
    let rows = choice_rows(
        &owned(&["normal", "soft_light", "hard_light"]),
        "soft_light",
    );
    assert_eq!(
        rows,
        vec![
            ("Normal".to_owned(), false),
            ("Soft light".to_owned(), true),
            ("Hard light".to_owned(), false),
        ]
    );
    // Nothing chosen yet (a value the catalog no longer offers): none ticked.
    assert!(choice_rows(&owned(&["a", "b"]), "c")
        .iter()
        .all(|(_, ticked)| !ticked));
}

fn option_name() -> impl Strategy<Value = String> {
    "[a-z]{1,10}(_[a-z]{1,6})?"
}

proptest! {
    #[test]
    fn compact_is_at_most_three_options_of_at_most_eight_letters(
        options in prop::collection::vec(option_name(), 0..7),
    ) {
        let short = options.iter().all(|option| option.chars().count() <= 8);
        prop_assert_eq!(choice_is_compact(&options), options.len() <= 3 && short);
    }

    #[test]
    fn a_row_is_ticked_exactly_where_its_option_is_the_one_chosen(
        options in prop::collection::vec(option_name(), 1..12),
        pick in any::<prop::sample::Index>(),
    ) {
        let chosen = options[pick.index(options.len())].clone();
        let rows = choice_rows(&options, &chosen);
        prop_assert_eq!(rows.len(), options.len());
        for ((words, ticked), option) in rows.iter().zip(&options) {
            prop_assert_eq!(*ticked, *option == chosen);
            prop_assert!(!words.trim().is_empty());
        }
        prop_assert!(rows.iter().any(|(_, ticked)| *ticked));
    }

    /// The menu's letter jump: the first letter of an option goes to a row
    /// that starts with it, the option's own or one before it in the cycle.
    #[test]
    fn a_letter_goes_to_an_option_starting_with_it(
        options in prop::collection::vec(option_name(), 1..12),
        pick in any::<prop::sample::Index>(),
    ) {
        let rows = choice_rows(&options, "");
        let words: Vec<&str> = rows.iter().map(|(words, _)| words.as_str()).collect();
        let enabled = vec![true; words.len()];
        let wanted = &options[pick.index(options.len())];
        let letter = wanted.chars().next().unwrap();
        let at = menu_letter(&words, &enabled, None, letter);
        prop_assert!(at.is_some());
        let at = at.unwrap();
        prop_assert!(options[at].starts_with(letter));
    }

    #[test]
    fn it_opens_over_its_button_only_where_under_it_is_short_and_over_it_roomier(
        top in 0.0f32..1200.0,
        tall in 16.0f32..40.0,
        height in 40.0f32..900.0,
        window in 200.0f32..1400.0,
    ) {
        let bottom = top + tall;
        let below = window - bottom - MARGIN;
        let above = top - MARGIN;
        let up = choice_menu_opens_up(top, bottom, height, window);
        if below >= height {
            prop_assert!(!up);
        }
        if up {
            prop_assert!(above > below);
        }
        if below < height && above > below {
            prop_assert!(up);
        }
    }
}

proptest! {
    /// Scrolled to show it, a button that fits the view lies whole inside it,
    /// clear of the fades, wherever the list could reach; one in sight stays.
    #[test]
    fn a_button_scrolled_to_shows_whole_and_one_in_sight_stays(
        view_top in 0.0f32..300.0,
        view_tall in 120.0f32..800.0,
        at in -2000.0f32..2000.0,
        tall in 16.0f32..40.0,
        offset in -1500.0f32..0.0,
    ) {
        let view = (view_top, view_top + view_tall);
        let item = (at, at + tall);
        let most = 4000.0;
        let shown = scroll_to_show(item, view, offset, most);
        prop_assert!((-most..=0.0).contains(&shown));
        let moved = shown - offset;
        let (top, bottom) = (item.0 + moved, item.1 + moved);
        let inside = item.0 >= view.0 + layout::TOP_FADE
            && item.1 <= view.1 - layout::BOTTOM_FADE;
        if inside {
            prop_assert_eq!(shown, offset);
        } else if shown > -most && shown < 0.0 {
            prop_assert!(top >= view.0 + layout::TOP_FADE - 0.01);
            prop_assert!(bottom <= view.1 - layout::BOTTOM_FADE + 0.01);
        }
    }
}

proptest! {
    /// Moved left as it must, the menu ends inside the window's margin
    /// where it fits, and never starts outside the margin on the left.
    #[test]
    fn the_menu_moved_left_stays_inside_the_window(
        window in 300.0f32..2000.0,
        at in 0.0f32..1.0,
        width in 180.0f32..600.0,
    ) {
        let left = space::S8 + at * (window - 2.0 * space::S8);
        let shift = choice_menu_shift(left, width, window);
        prop_assert!(shift >= 0.0);
        prop_assert!(left - shift >= space::S8 - 0.01);
        if width <= window - 2.0 * space::S8 {
            prop_assert!(left - shift + width <= window - space::S8 + 0.01);
        }
    }
}

proptest! {
    /// A menu stays open exactly while the body is scrolled as it was when
    /// it opened and some of its button shows in the body's view.
    #[test]
    fn a_menu_stays_only_unscrolled_over_a_button_in_sight(
        view_top in 0.0f32..300.0,
        view_tall in 120.0f32..800.0,
        at in -1500.0f32..1500.0,
        tall in 16.0f32..40.0,
        opened in -1500.0f32..0.0,
        rolled in -400.0f32..400.0,
    ) {
        let view = (view_top, view_top + view_tall);
        let button = (at, at + tall);
        let in_sight = button.1 > view.0 && button.0 < view.1;
        prop_assert_eq!(choice_menu_stays(opened, opened, button, view), in_sight);
        if rolled.abs() >= 1.0 {
            prop_assert!(!choice_menu_stays(opened, opened + rolled, button, view));
        }
    }

    /// Worked out on what the choice holds now, a menu's ticked row is the
    /// one row that sends nothing: any other sends, whichever was ticked as
    /// the menu opened; keyed, none sends.
    #[test]
    fn the_row_ticked_now_is_the_one_that_sends_nothing(
        options in prop::collection::vec(option_name(), 2..12),
        now in any::<prop::sample::Index>(),
        keyed in any::<bool>(),
    ) {
        let held = &options[now.index(options.len())];
        for ((_, ticked), option) in choice_rows(&options, held).iter().zip(&options) {
            prop_assert_eq!(choice_sends(option, held, keyed), !keyed && !*ticked);
        }
    }
}

// ---------------------------------------------------------------- typing in the Inspector

/// kerfd as it is built beside the editor: `cargo build -p kerf-runtime --bin kerfd`.
fn kerfd() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(format!("kerfd{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "build kerfd first: cargo build -p kerf-runtime --bin kerfd"
    );
    path
}

/// The Inspector in a window while a clip is chosen, as the workspace shows it, beside somewhere
/// else that takes the keys when it is pressed, as the timeline does.
struct Beside {
    session: Entity<Session>,
    inspector: Entity<InspectorView>,
    elsewhere: FocusHandle,
}

impl Render for Beside {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = !self.session.read(cx).selection.is_empty();
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(px(320.0))
                    .h_full()
                    .children(chosen.then(|| self.inspector.clone())),
            )
            .child(div().flex_1().h_full().track_focus(&self.elsewhere))
    }
}

/// Runs the window until `done` holds of the session, as kerfd answers from its own process.
fn run_until(
    cx: &mut VisualTestContext,
    session: &Entity<Session>,
    what: &str,
    done: impl Fn(&Session) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if session.read_with(cx, |session, _| done(session)) {
            return;
        }
        assert!(Instant::now() < deadline, "kerfd never showed {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn timeline_clip<'a>(session: &'a Session, id: &str) -> Option<&'a Clip> {
    session
        .timeline()
        .tracks
        .iter()
        .flat_map(|track| &track.clips)
        .find(|clip| clip.id == id)
}

fn name_of<'a>(session: &'a Session, clip: &str) -> Option<&'a str> {
    timeline_clip(session, clip).and_then(|clip| clip.name.as_deref())
}

fn prop_of(session: &Session, clip: &str, prop: &str) -> Option<Value> {
    timeline_clip(session, clip).and_then(|clip| clip.props.get(prop).cloned())
}

fn choose_clip(cx: &mut VisualTestContext, session: &Entity<Session>, id: &str) {
    session.update(cx, |session, cx| {
        session.selection = vec![id.to_owned()];
        cx.notify();
    });
    cx.run_until_parked();
}

/// What a field holds, typed over with `text`, as Select All then typing does.
fn type_over(cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(text);
}

/// What is typed in a field is sent when the field is left by Tab or by a press elsewhere (another
/// clip among them), not only by Return: the window draws the Inspector again as the keys move,
/// before it hears the field was left, and that drawing must not put back what the clip had.
#[gpui::test]
fn leaving_an_inspector_field_by_tab_or_a_press_elsewhere_sends_what_was_typed(
    cx: &mut TestAppContext,
) {
    let project = tempfile::Builder::new()
        .prefix("kq-inspect")
        .tempdir()
        .unwrap();
    let folder = project.path().to_owned();
    cx.update(|cx| cx.bind_keys(input::key_bindings()));
    let (window, cx) = cx.add_window_view(|window, cx| {
        let session = Session::open(Some(folder), Some(kerfd()), Prefs::default(), window, cx);
        let inspector = cx.new(|cx| InspectorView::new(session.clone(), window, cx));
        Beside {
            session,
            inspector,
            elsewhere: cx.focus_handle(),
        }
    });
    // A window not in front hears no field left.
    cx.update(|window, _| window.activate_window());
    let session = window.read_with(cx, |window, _| window.session.clone());
    run_until(cx, &session, "it was ready", Session::connected);
    // A title and a solid, as the Titles browser adds them.
    let added = session.update(cx, |session, _| {
        session.edit(vec![
            json!({"op": "add-track", "kind": "video", "id": "V1"}),
            json!({"op": "insert", "generator": {"fx": "title", "params": {"text": "Atlas"}},
                   "at": 0, "frames": 50, "tracks": ["V1"], "ids": ["title"], "mode": "overwrite"}),
            json!({"op": "insert", "generator": {"fx": "solid", "params": {"color": "#ff0000"}},
                   "at": 50, "frames": 25, "tracks": ["V1"], "ids": ["solid"], "mode": "overwrite"}),
        ])
    });
    assert!(added);
    run_until(cx, &session, "the clips", |session| {
        timeline_clip(session, "title").is_some() && timeline_clip(session, "solid").is_some()
    });

    // The clip's name, the Inspector's first stop, left by Tab.
    choose_clip(cx, &session, "title");
    cx.update(|window, _| window.focus_next());
    type_over(cx, "Opening");
    cx.simulate_keystrokes("tab");
    run_until(cx, &session, "the name typed", |session| {
        name_of(session, "title") == Some("Opening")
    });
    // Tab went on to the title's text, left by a press elsewhere.
    type_over(cx, "Sunrise");
    let press = cx
        .update(|window, _| window.bounds().size)
        .map(|side| side * 0.75);
    cx.simulate_click(point(press.width, press.height), Modifiers::none());
    run_until(cx, &session, "the text typed", |session| {
        prop_of(session, "title", "text") == Some(json!("Sunrise"))
    });
    // A solid's colour, its one field after the name, left by Tab.
    choose_clip(cx, &session, "solid");
    cx.update(|window, _| window.focus_next());
    cx.simulate_keystrokes("tab");
    type_over(cx, "#00ff00");
    cx.simulate_keystrokes("tab");
    run_until(cx, &session, "the colour typed", |session| {
        prop_of(session, "solid", "color") == Some(json!("#00ff00"))
    });
    // The solid's name, left by a press on another clip, which takes the keys and chooses that
    // clip at once, as the timeline does: the name goes to the clip it was typed for.
    let elsewhere = window.read_with(cx, |window, _| window.elsewhere.clone());
    cx.update(|window, _| {
        window.focus(&elsewhere);
        window.focus_next();
    });
    type_over(cx, "Red");
    cx.update(|window, cx| {
        window.focus(&elsewhere);
        session.update(cx, |session, cx| {
            session.selection = vec!["title".to_owned()];
            cx.notify();
        });
    });
    run_until(
        cx,
        &session,
        "the name typed before another clip",
        |session| name_of(session, "solid") == Some("Red"),
    );
    session.read_with(cx, |session, _| {
        assert_eq!(name_of(session, "title"), Some("Opening"));
    });
    // The title's name, left as the Inspector goes (a press on an empty lane chooses nothing): it
    // is sent as it is left, and an Undo of it made meanwhile stays undone as the title is chosen
    // again. kerfd takes edits in order, so the text sent by Return after that comes after any
    // name sent again.
    cx.update(|window, _| {
        window.focus(&elsewhere);
        window.focus_next();
    });
    type_over(cx, "Closing");
    cx.update(|window, cx| {
        window.focus(&elsewhere);
        session.update(cx, |session, cx| {
            session.selection.clear();
            cx.notify();
        });
    });
    run_until(
        cx,
        &session,
        "the name typed as the Inspector went",
        |session| name_of(session, "title") == Some("Closing"),
    );
    session.update(cx, |session, _| session.undo());
    run_until(cx, &session, "the name undone", |session| {
        name_of(session, "title") == Some("Opening")
    });
    choose_clip(cx, &session, "title");
    cx.update(|window, _| {
        window.focus(&elsewhere);
        window.focus_next();
    });
    cx.simulate_keystrokes("tab");
    type_over(cx, "Dusk");
    cx.simulate_keystrokes("enter");
    run_until(cx, &session, "the text sent by Return", |session| {
        prop_of(session, "title", "text") == Some(json!("Dusk"))
    });
    session.read_with(cx, |session, _| {
        assert_eq!(name_of(session, "title"), Some("Opening"));
    });
}

// ---------------------------------------------------------------- the master output

#[test]
fn the_master_goes_to_deliveries_unless_the_project_keeps_another_place() {
    let project = Path::new("/films/atlas");
    assert_eq!(
        master_request(Some(project), None),
        command(
            json!({"type": "export", "quality": "master", "output": "deliveries/", "name": "atlas_master"})
        )
    );
    let kept = MasterOutput {
        folder: Some("/Volumes/out/".into()),
        name: Some("  Atlas Final ".into()),
        codec: Some("hevc".into()),
    };
    assert_eq!(
        master_request(Some(project), Some(&kept)),
        command(
            json!({"type": "export", "quality": "master", "codec": "hevc", "output": "/Volumes/out/", "name": "Atlas Final"})
        )
    );
    assert_eq!(
        master_goes(Some(project), Some(&kept)).0,
        "/Volumes/out/Atlas Final.mp4"
    );
    // ProRes is a master's own codec and goes without saying; an empty
    // folder is where kerfd puts renders.
    let own = MasterOutput {
        folder: Some(String::new()),
        name: None,
        codec: Some("prores_hq".into()),
    };
    assert_eq!(
        master_request(Some(project), Some(&own)),
        command(json!({"type": "export", "quality": "master", "name": "atlas_master"}))
    );
    assert_eq!(master_goes(Some(project), Some(&own)).0, "atlas_master.mov");
    assert_eq!(
        master_request(None, None),
        command(json!({"type": "export", "quality": "master"}))
    );
}

/// A project that keeps its master in `folder` as `name`, in `codec`.
fn keeps(folder: Option<&str>, name: Option<&str>, codec: Option<&str>) -> MasterOutput {
    MasterOutput {
        folder: folder.map(str::to_owned),
        name: name.map(str::to_owned),
        codec: codec.map(str::to_owned),
    }
}

#[test]
fn every_way_of_exporting_the_master_sends_where_the_project_keeps_it() {
    let project = Path::new("/films/harbour");
    let mut prefs = Prefs::default();
    // Nothing kept: deliveries/, as the toolbar sent it before.
    assert_eq!(
        request(Some(project), &prefs, RenderQuality::Master),
        kerf_editor::delivery::export_request(Some(project), RenderQuality::Master)
    );
    prefs.master_outputs.insert(
        kerf_editor::prefs::project_key(project),
        keeps(Some("masters"), Some("Harbour final"), Some("h264")),
    );
    assert_eq!(
        request(Some(project), &prefs, RenderQuality::Master),
        command(
            json!({"type": "export", "quality": "master", "codec": "h264", "output": "masters", "name": "Harbour final"})
        )
    );
    // A review copy, a full render and a preview are not the master's.
    for quality in [
        RenderQuality::Review,
        RenderQuality::Full,
        RenderQuality::Preview,
    ] {
        assert_eq!(
            request(Some(project), &prefs, quality.clone()),
            kerf_editor::delivery::export_request(Some(project), quality)
        );
    }
    // Another project keeps nothing.
    assert_eq!(
        serde_json::to_value(request(
            Some(Path::new("/films/atlas")),
            &prefs,
            RenderQuality::Master
        ))
        .unwrap()["output"],
        "deliveries/"
    );
}

#[test]
fn the_master_says_what_it_is_made_as_and_where_it_goes() {
    let project = Some(Path::new("/films/harbour"));
    let hdr = keeps(Some("masters"), None, Some("hevc_hdr"));
    assert_eq!(
        master_goes(project, Some(&hdr)),
        (
            "masters/harbour_master.mp4".to_owned(),
            "HEVC 10-bit HDR in MP4, with its sound".to_owned()
        )
    );
    assert_eq!(master_makes(Some(&hdr)), "HEVC 10-bit HDR");
    assert_eq!(master_makes(None), Quality::Master.makes());
    assert_eq!(
        master_makes(Some(&keeps(None, None, Some("prores_hq")))),
        "ProRes 422 HQ"
    );
    assert_eq!(
        master_makes(Some(&keeps(None, None, Some("h264")))),
        "H.264"
    );
    // A codec kerfd renders no master in is not called ProRes.
    let odd = keeps(None, None, Some("av1"));
    assert_eq!(master_makes(Some(&odd)), "av1");
    let (goes, makes) = master_goes(project, Some(&odd));
    assert_eq!(goes, "deliveries/harbour_master");
    assert!(makes.starts_with("av1"), "{makes}");
    assert!(!makes.contains("ProRes"), "{makes}");
}

#[test]
fn export_says_where_one_press_puts_the_chosen_quality() {
    let project = Some(Path::new("/films/harbour"));
    let kept = keeps(Some("masters"), None, None);
    assert_eq!(
        export_into(Quality::Master, project, None).as_deref(),
        Some("Into deliveries/")
    );
    assert_eq!(
        export_into(Quality::Master, project, Some(&kept)).as_deref(),
        Some("Into masters/")
    );
    assert_eq!(
        export_into(Quality::Draft, project, Some(&kept)).as_deref(),
        Some("Into deliveries/")
    );
    // Where kerfd puts renders has no folder to name.
    let anywhere = keeps(Some(""), None, None);
    assert_eq!(export_into(Quality::Master, project, Some(&anywhere)), None);
    assert_eq!(export_into(Quality::Draft, None, None), None);
}

#[test]
fn deliver_says_where_the_master_and_the_review_copy_go() {
    let project = Some(Path::new("/films/harbour"));
    assert_eq!(
        deliver_said(project, None),
        "The master and a review copy go to deliveries/ in the project, numbered when one is there already."
    );
    assert_eq!(
        deliver_said(project, Some(&keeps(Some("masters"), None, None))),
        "The master goes to masters/ and a review copy to deliveries/ in the project, each numbered when one is there already."
    );
    assert_eq!(
        deliver_said(project, Some(&keeps(Some("/Volumes/out/"), None, None))),
        "The master goes to /Volumes/out/ and a review copy to deliveries/ in the project, each numbered when one is there already."
    );
    assert_eq!(
        deliver_said(project, Some(&keeps(Some(""), None, None))),
        "The master goes where Kerf puts renders and a review copy to deliveries/ in the project, each numbered when one is there already."
    );
}

proptest! {
    #[test]
    fn the_master_request_names_what_the_deliver_row_says(
        project in "[a-zA-Z0-9 _-]{1,20}",
        folder in prop::option::of("[a-z/]{0,12}"),
        name in prop::option::of("[a-zA-Z0-9 ]{0,12}"),
        codec in prop::option::of(prop::sample::select(vec!["prores_hq", "h264", "hevc", "hevc_hdr", ""])),
    ) {
        let path = PathBuf::from("/films").join(project.trim());
        prop_assume!(path.file_name().is_some());
        let kept = MasterOutput {
            folder,
            name,
            codec: codec.map(str::to_owned),
        };
        let command = serde_json::to_value(master_request(Some(&path), Some(&kept))).unwrap();
        let (goes, _) = master_goes(Some(&path), Some(&kept));
        // The file the row names is the request's name, in the codec's container.
        let extension = match command["codec"].as_str() {
            None => "mov",
            Some(_) => "mp4",
        };
        let file = format!("{}.{extension}", command["name"].as_str().unwrap());
        prop_assert!(goes.ends_with(&file), "{} does not end with {}", goes, file);
        match command["output"].as_str() {
            Some(folder) => prop_assert!(goes.starts_with(folder)),
            None => prop_assert_eq!(&goes, &file),
        }
        // ProRes goes without saying; kerfd's default for a master.
        prop_assert_ne!(command["codec"].as_str(), Some("prores_hq"));
        // With nothing kept, the toolbar's request is the master's.
        prop_assert_eq!(
            master_request(Some(&path), None),
            kerf_editor::delivery::export_request(Some(&path), RenderQuality::Master)
        );
    }
}

// ---------------------------------------------------------------- Collab's words

#[test]
fn publish_starts_from_the_folders_name_and_shares_three_ways() {
    assert_eq!(publish_name("My Film (v2)"), "My-Film-v2-");
    assert_eq!(publish_name("atlas.cut_1"), "atlas.cut_1");
    assert_eq!(publish_name("가편집"), "-");
    assert_eq!(
        publish_command(PublishWay::GitHub, " atlas ", false, ""),
        Some(command(
            json!({"type": "collab_publish", "github": {"private": false, "name": "atlas"}})
        ))
    );
    assert_eq!(
        publish_command(PublishWay::GitHub, "", true, ""),
        Some(command(
            json!({"type": "collab_publish", "github": {"private": true}})
        ))
    );
    assert_eq!(publish_command(PublishWay::Remote, "x", true, "  "), None);
    assert_eq!(
        publish_command(PublishWay::Remote, "x", true, " git@gitlab.com:j/a.git "),
        Some(command(
            json!({"type": "collab_publish", "remote": "git@gitlab.com:j/a.git"})
        ))
    );
    assert_eq!(
        publish_command(PublishWay::Local, "x", true, "y"),
        Some(command(json!({"type": "collab_publish"})))
    );
}

#[test]
fn a_pull_request_is_titled_from_its_branch_and_said_with_its_url() {
    assert_eq!(pull_request_title("trim-the_opening"), "Trim the opening");
    assert_eq!(pull_request_title(""), "");
    assert_eq!(pull_request_command("  ", "body"), None);
    assert_eq!(
        pull_request_command("Trim", "  "),
        Some(command(
            json!({"type": "collab_pull_request", "title": "Trim"})
        ))
    );
    assert_eq!(
        pull_request_command("Trim", "Shorter\nopening"),
        Some(command(
            json!({"type": "collab_pull_request", "title": "Trim", "body": "Shorter\nopening"})
        ))
    );
    assert_eq!(
        pull_request_said("https://github.com/j/a/pull/4", true),
        "Opened a pull request: https://github.com/j/a/pull/4"
    );
    assert_eq!(
        pull_request_said("https://github.com/j/a/compare/trim?expand=1", false),
        "Opened GitHub's page for a new pull request: https://github.com/j/a/compare/trim?expand=1"
    );
    assert_eq!(
        pull_request_said("https://github.com/j/a/pull/3", false),
        "This branch already has a pull request: https://github.com/j/a/pull/3"
    );
}

#[test]
fn what_a_sync_brought_is_said_and_left_in_activity() {
    // A report as the bridge sends one, from the JSON it is written in.
    let read = |written: serde_json::Value| -> kerf_bridge::event::SyncReport {
        serde_json::from_value(written).expect("a report that reads")
    };
    let quiet = read(json!({"incoming": [], "conflicts": [], "pushed": false}));
    let report = read(json!({
        "incoming": [{"author": "jun", "subject": "Trimmed c3"}, {"author": "mi", "subject": "Title"}],
        "conflicts": [
            {"path": "project.kerf", "what": "c3's out point", "kept": "yours"},
            {"path": "", "what": "the title's words", "kept": "theirs"},
        ],
        "renamed": [["c4", "c7"]],
        "pushed": true,
    }));
    assert_eq!(
        sync_lines(&report),
        [
            "jun · Trimmed c3",
            "mi · Title",
            "Kept yours · project.kerf: c3's out point",
            "Kept theirs · the title's words",
            "Renumbered · c4 → c7",
        ]
    );
    assert_eq!(
        sync_said(&report, false).as_deref(),
        Some("Synced: 2 commits in, pushed, 1 change kept yours, 1 change kept theirs")
    );
    assert_eq!(
        sync_said(&quiet, false),
        None,
        "a quiet live cycle says nothing"
    );
    assert_eq!(
        sync_said(&quiet, true).as_deref(),
        Some("Synced: up to date")
    );
    // Fields said otherwise than the bridge reads them each read as their default, and the
    // rest of the report is still said, as the tray said it when it read the report by key.
    let askew = read(json!({
        "committed": true,
        "incoming": [{"author": "jun", "subject": "Trimmed c3"}],
        "conflicts": [{"path": "project", "what": "c3's out point", "kept": null}],
        "renamed": [["c4"]],
        "pushed": null,
    }));
    assert_eq!(
        sync_lines(&askew),
        [
            "jun · Trimmed c3",
            "Kept yours · project: c3's out point",
            "Renumbered · c4 → ",
        ]
    );
    assert_eq!(
        sync_said(&askew, false).as_deref(),
        Some("Synced: 1 commit in, 1 change kept yours")
    );
}

// ---------------------------------------------------------------- Preflight

fn row(code: &str, passed: bool) -> Value {
    json!({"code": code, "name": code.to_uppercase(), "passed": passed, "value": "1", "limit": "≤ 2"})
}

proptest! {
    #[test]
    fn preflight_shows_every_failure_it_can_then_passes(passes in prop::collection::vec(any::<bool>(), 0..14)) {
        let summary = json!({"rows": passes
            .iter()
            .enumerate()
            .map(|(index, passed)| row(&format!("c{index}"), *passed))
            .collect::<Vec<_>>()});
        let failed = passes.iter().filter(|passed| !**passed).count();
        let rows = preflight_rows(&summary, 4, 6);
        let shown_failed = rows.iter().filter(|row| !row.passed).count();
        prop_assert_eq!(shown_failed, failed.min(6));
        prop_assert_eq!(preflight_hidden(&summary, 6), failed.saturating_sub(6));
        prop_assert_eq!(rows.len(), shown_failed.max(4.min(passes.len())));
        // In the summary's order.
        let order: Vec<usize> = rows.iter().map(|row| row.code[1..].parse().unwrap()).collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        prop_assert_eq!(order, sorted);
    }
}

#[test]
fn the_format_banks_arrows_go_to_the_format_beside_and_stop_at_the_ends() {
    let landscape = format_index(1920, 1080);
    let portrait = format_index(1080, 1350);
    assert_eq!(landscape, Some(0));
    assert_eq!(portrait, Some(3));
    assert_eq!(format_beside(landscape, true), Some(1));
    assert_eq!(format_beside(landscape, false), None);
    assert_eq!(format_beside(portrait, true), None);
    assert_eq!(format_beside(portrait, false), Some(2));
    // A size of its own: Right goes to the first, Left to the last.
    assert_eq!(format_index(1280, 720), None);
    assert_eq!(format_beside(None, true), Some(0));
    assert_eq!(format_beside(None, false), Some(3));
}

proptest! {
    /// From any format, an arrow goes one along or stays, never off the bank.
    #[test]
    fn an_arrow_moves_one_format_at_most(at in proptest::option::of(0usize..6), forward: bool) {
        match format_beside(at, forward) {
            Some(next) => {
                prop_assert!(next < 4);
                if let Some(at) = at {
                    prop_assert_eq!(next as i64 - at as i64, if forward { 1 } else { -1 });
                }
            }
            None => prop_assert!(at.is_some()),
        }
    }
}

#[test]
fn a_master_and_a_draft_go_into_deliveries_named_after_the_folder() {
    let project = Path::new("/films/Atlas");
    assert_eq!(
        export_request(Some(project), RenderQuality::Master),
        command(
            json!({"type": "export", "quality": "master", "output": DELIVERIES, "name": "Atlas_master"})
        )
    );
    assert_eq!(
        export_request(Some(project), Quality::Draft.wire()),
        command(
            json!({"type": "export", "quality": "review", "codec": "h264", "output": DELIVERIES,
            "name": "Atlas_draft"})
        )
    );
    // Other qualities, and a project with no folder name, go where kerfd puts them.
    assert_eq!(
        export_request(Some(project), RenderQuality::Preview),
        command(json!({"type": "export", "quality": "preview"}))
    );
    assert_eq!(
        export_request(None, RenderQuality::Review),
        command(json!({"type": "export", "quality": "review", "codec": "h264"}))
    );
}

#[test]
fn the_export_button_names_its_quality_and_why_it_cannot_render() {
    assert_eq!(Quality::default(), Quality::Master);
    assert_eq!(Quality::Master.wire(), RenderQuality::Master);
    assert_eq!(Quality::Draft.name(), "Draft");
    assert_eq!(Quality::Master.makes(), "ProRes 422 HQ");
    assert_eq!(export_tip(Quality::Master, None), "Export master");
    assert_eq!(
        export_tip(Quality::Draft, Some("Kerf is offline")),
        "Export draft · Kerf is offline"
    );
    assert_eq!(export_tip(Quality::Draft, Some("  ")), "Export draft");
}

proptest! {
    #[test]
    fn every_delivery_is_the_folders_name_and_its_kind(folder in "[A-Za-z0-9 _-]{1,24}") {
        prop_assume!(!folder.trim().is_empty());
        let project = PathBuf::from("/work").join(&folder);
        for (quality, suffix) in [
            (RenderQuality::Master, "master"),
            (RenderQuality::Review, "draft"),
        ] {
            let request = serde_json::to_value(export_request(Some(&project), quality)).unwrap();
            prop_assert_eq!(request["output"].as_str(), Some(DELIVERIES));
            let name = format!("{folder}_{suffix}");
            prop_assert_eq!(request["name"].as_str(), Some(name.as_str()));
        }
    }
}

/// What File ▸ Export… asks the registry for: the chosen quality as
/// `file.export` names it, rendered where kerfd puts it, not the toolbar's
/// delivery into deliveries/.
fn file_export(quality: Quality) -> serde_json::Value {
    kerf_bridge::commands_run(&json!({
        "command": "file.export",
        "context": {"connected": true},
        "params": {"quality": quality.file_wire()},
    }))
}

#[test]
fn file_export_renders_the_chosen_quality_where_kerfd_puts_it() {
    assert_eq!(
        file_export(Quality::Master),
        json!({"request": {"type": "export", "quality": "full"}})
    );
    assert_eq!(
        file_export(Quality::Draft),
        json!({"request": {"type": "export", "quality": "preview"}})
    );
    // The toolbar's Export, at the same choice, delivers a master instead.
    let toolbar = serde_json::to_value(export_request(
        Some(Path::new("/films/Atlas")),
        Quality::Master.wire(),
    ))
    .unwrap();
    assert_eq!(toolbar["quality"], "master");
    assert_eq!(toolbar["output"], DELIVERIES);
}
