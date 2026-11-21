//! The window, the picture and the sidebar, through their public functions:
//! where the keys settle, which panes come in when one is asked for, the
//! app's menu and how its rows stand, the menu bar's greyed and ticked rows,
//! the user guide's URL, the title's tooltip and what the toolbar says
//! renders, Focus Region's box, range, edit and tracking, the transport and
//! the working chip, Going home and the change reel's compare.

use gpui::KeyContext;
use kerf_editor::commands::{self, menus_with_states, MenuStates};
use kerf_editor::focus::{
    crop_box, crop_props, ffmpeg_bin, first_cut, focus_ops, readable_frame, shown_time,
    track_region, typed_time, Region, NO_FRAME, TRACK_SIZE,
};
use kerf_editor::model::{Clip, RenderJob, Timeline};
use kerf_editor::prefs::Keymap;
use kerf_editor::sidebar::{self, app_rows, AppDoes, AppMenuInput, AppRow};
use kerf_editor::stage::{
    base64_decode, compare_at, kerf_cutting, placed_clips, poster_source, pulse_level,
    releases_cover, transport_reasons, transport_tip, TRANSPORT,
};
use kerf_editor::store::Preview;
use kerf_editor::workspace::{
    app_menu_place, app_menu_room, edited_ago_in, file_url, holds_keys_itself, home_flight_at,
    panes_opening, running_words, settle_keys, sync_params, sync_question, title_tip,
    user_guide_in, Holder, KeysGo,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

// ------------------------------------------------------------ the keys

#[test]
fn the_keys_settle_on_the_timeline_unless_someone_is_typing() {
    // With the timeline in view, what nothing in particular holds goes to it.
    assert_eq!(settle_keys(true, Holder::Nothing), KeysGo::Timeline);
    assert_eq!(settle_keys(true, Holder::Other), KeysGo::Timeline);
    assert_eq!(settle_keys(true, Holder::Timeline), KeysGo::Stay);
    // A field, a menu, a panel over the window keep them.
    assert_eq!(settle_keys(true, Holder::Busy), KeysGo::Stay);
    assert_eq!(settle_keys(false, Holder::Busy), KeysGo::Stay);
    // With no timeline in view they wait with the window, and a timeline
    // folded away gives them back.
    assert_eq!(settle_keys(false, Holder::Nothing), KeysGo::Window);
    assert_eq!(settle_keys(false, Holder::Timeline), KeysGo::Window);
    assert_eq!(settle_keys(false, Holder::Other), KeysGo::Stay);
}

#[test]
fn a_field_a_menu_or_a_panel_keeps_the_keys() {
    let stack = |contexts: &[&str]| -> Vec<KeyContext> {
        contexts
            .iter()
            .map(|context| KeyContext::parse(context).unwrap())
            .collect()
    };
    assert!(holds_keys_itself(&stack(&["Workspace", "TextInput"])));
    assert!(holds_keys_itself(&stack(&["Workspace", "KitMenu"])));
    assert!(holds_keys_itself(&stack(&[
        "Workspace",
        "Sidebar",
        "SidebarMenu"
    ])));
    assert!(holds_keys_itself(&stack(&["Workspace", "Settings"])));
    assert!(holds_keys_itself(&stack(&[
        "Workspace",
        "KitPanel FocusRegion"
    ])));
    assert!(holds_keys_itself(&stack(&["Workspace", "CommandPalette"])));
    // The window, the timeline, a button of the kit: they do not.
    assert!(!holds_keys_itself(&stack(&["Workspace"])));
    assert!(!holds_keys_itself(&stack(&["Workspace", "Timeline"])));
    assert!(!holds_keys_itself(&stack(&["Workspace", "KitControl"])));
}

// ------------------------------------------------------------ the panes

#[test]
fn a_pane_asked_for_with_the_panel_closed_comes_in_alone() {
    // Closed: only what is asked for.
    assert_eq!(
        panes_opening(false, (true, true), (false, true)),
        (false, true)
    );
    assert_eq!(
        panes_opening(false, (true, true), (true, false)),
        (true, false)
    );
    // Open: it joins what shows.
    assert_eq!(
        panes_opening(true, (false, true), (true, false)),
        (true, true)
    );
    // The panel's button asks for neither in particular: both.
    assert_eq!(
        panes_opening(false, (false, false), (false, false)),
        (true, true)
    );
}

proptest! {
    #[test]
    fn the_panel_always_shows_what_was_asked_and_something(
        open in any::<bool>(),
        shown in any::<(bool, bool)>(),
        asked in any::<(bool, bool)>(),
    ) {
        let (picture, timeline) = panes_opening(open, shown, asked);
        prop_assert!(picture || timeline);
        prop_assert!(picture || !asked.0);
        prop_assert!(timeline || !asked.1);
        if !open && asked != (false, false) {
            prop_assert_eq!((picture, timeline), asked);
        }
        // Asking again changes nothing.
        prop_assert_eq!(panes_opening(true, (picture, timeline), asked), (picture, timeline));
    }
}

// ------------------------------------------------------------ the app's menu

fn input<'a>(
    commands: &'a [commands::Command],
    menus: &'a [String],
    states: &'a Value,
    checked: &'a BTreeSet<String>,
    collab: Option<&'a Value>,
    menu_bar: bool,
) -> AppMenuInput<'a> {
    AppMenuInput {
        signed_in: false,
        keymap: Keymap::Premiere,
        commands,
        menus,
        states,
        checked,
        zoom: 0.5,
        collab,
        revision: 22,
        menu_bar,
        recent: &[],
        current: None,
    }
}

fn find<'a>(rows: &'a [AppRow], words: &str) -> &'a AppRow {
    rows.iter()
        .find(|row| row.words == words)
        .unwrap_or_else(|| {
            panic!(
                "no row {words:?} in {:?}",
                rows.iter().map(|row| &row.words).collect::<Vec<_>>()
            )
        })
}

#[test]
fn the_app_menus_view_holds_the_panes_the_tools_the_zoom_the_keyboard_and_the_sync() {
    let commands = commands::catalog("premiere");
    let states = json!({"view.timeline": null, "view.picture": null});
    let checked: BTreeSet<String> = ["view.sidebar", "view.followKerf"]
        .map(str::to_owned)
        .into();
    let rows = app_rows(&input(&commands, &[], &states, &checked, None, true));
    let view = find(&rows, "View");
    let words: Vec<&str> = view.rows.iter().map(|row| row.words.as_str()).collect();
    assert_eq!(
        words,
        [
            "Sidebar",
            "Picture",
            "Timeline",
            "Follow Kerf",
            "Rectangle mask",
            "Ellipse mask",
            "Pen mask",
            "Safe Zones",
            "Viewer Zoom",
            "Keyboard",
            "Sync"
        ]
    );
    // The switches on are ticked, with their keys.
    assert!(find(&view.rows, "Sidebar").checked);
    assert!(!find(&view.rows, "Picture").checked);
    assert!(find(&view.rows, "Follow Kerf").checked);
    assert!(find(&view.rows, "Sidebar").key.is_some());
    assert_eq!(
        find(&view.rows, "Pen mask").does,
        AppDoes::Mask(kerf_editor::stage::MaskTool::Pen)
    );
    // The zoom in use and the keymap in use are ticked.
    let zoom = find(&view.rows, "Viewer Zoom");
    let ticked: Vec<&str> = zoom
        .rows
        .iter()
        .filter(|row| row.checked)
        .map(|row| row.words.as_str())
        .collect();
    assert_eq!(ticked, ["50%"]);
    assert_eq!(find(&zoom.rows, "Fit").does, AppDoes::Zoom(0));
    let keyboard = find(&view.rows, "Keyboard");
    let ticked: Vec<&AppDoes> = keyboard
        .rows
        .iter()
        .filter(|row| row.checked)
        .map(|row| &row.does)
        .collect();
    assert_eq!(ticked, [&AppDoes::Keymap(Keymap::Premiere)]);
    // With a menu bar the registry's menus are not repeated.
    assert!(!rows.iter().any(|row| row.words == "File"));
}

#[test]
fn without_a_menu_bar_the_registrys_menus_are_in_the_app_menu() {
    let commands = commands::catalog("kerf");
    let menus: Vec<String> = ["File", "Edit", "Help"].map(str::to_owned).to_vec();
    let states = json!({"edit.undo": "Nothing to undo"});
    let checked = BTreeSet::new();
    let rows = app_rows(&input(&commands, &menus, &states, &checked, None, false));
    let file = find(&rows, "File");
    assert!(file.separated);
    // Open Recent follows Open Project, with nothing recent said so.
    let at = file
        .rows
        .iter()
        .position(|row| row.does == AppDoes::Command("file.openProject".into()))
        .unwrap();
    assert_eq!(file.rows[at + 1].words, "Open Recent");
    assert_eq!(file.rows[at + 1].rows[0].words, "No Recent Projects");
    assert!(file.rows[at + 1].rows[0].disabled);
    // What cannot run is greyed, with why.
    let edit = find(&rows, "Edit");
    let undo = edit
        .rows
        .iter()
        .find(|row| row.does == AppDoes::Command("edit.undo".into()))
        .unwrap();
    assert!(undo.disabled);
    assert_eq!(undo.tip.as_deref(), Some("Nothing to undo"));
    assert!(rows.iter().any(|row| row.words == "Help"));
}

#[test]
fn the_sync_says_how_it_stands_and_switches_branches() {
    assert_eq!(
        sidebar::sync_summary(None, 4),
        "Revision 4 · not shared yet"
    );
    let status = json!({
        "repository": true, "branch": "main", "branches": ["main", "trailer"],
        "behind": 1, "ahead": 2, "conflicted": false, "live": true,
    });
    assert_eq!(
        sidebar::sync_summary(Some(&status), 22),
        "Revision 22 · 1 commit to pull · 2 commits to push"
    );
    let commands = commands::catalog("kerf");
    let states = json!({});
    let checked: BTreeSet<String> = ["collab.live".to_owned()].into();
    let rows = app_rows(&input(
        &commands,
        &[],
        &states,
        &checked,
        Some(&status),
        true,
    ));
    let sync = find(&find(&rows, "View").rows, "Sync");
    assert!(sync.rows[0].disabled);
    assert_eq!(find(&sync.rows, "main").does, AppDoes::Nothing);
    assert!(find(&sync.rows, "main").checked);
    assert_eq!(
        find(&sync.rows, "trailer").does,
        AppDoes::Branch("trailer".into())
    );
    let live = sync
        .rows
        .iter()
        .find(|row| row.does == AppDoes::Command("collab.live".into()))
        .unwrap();
    assert!(live.checked);
    // No repository yet: only Publish.
    let none = app_rows(&input(&commands, &[], &states, &checked, None, true));
    let sync = find(&find(&none, "View").rows, "Sync");
    assert_eq!(sync.rows.len(), 2);
    assert_eq!(sync.rows[1].does, AppDoes::Command("collab.publish".into()));
}

#[test]
fn every_row_of_the_app_menu_does_something_or_says_how_things_stand() {
    let commands = commands::catalog("kerf");
    let menus: Vec<String> = kerf_bridge::commands_catalog("kerf")["menus"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let states = json!({});
    let checked = BTreeSet::new();
    let mut input = input(&commands, &menus, &states, &checked, None, false);
    input.signed_in = true;
    let rows = app_rows(&input);
    fn walk(rows: &[AppRow]) {
        for row in rows {
            if row.rows.is_empty() {
                assert!(
                    row.does.action().is_some() || row.disabled,
                    "{} does nothing",
                    row.words
                );
            } else {
                walk(&row.rows);
            }
        }
    }
    walk(&rows);
    assert_eq!(rows.last().unwrap().does, AppDoes::SignOut);
}

// ------------------------------------------------------------ the menu bar

fn menu_items(menus: &[gpui::Menu], name: &str) -> Vec<(String, bool)> {
    let menu = menus.iter().find(|menu| menu.name == name).unwrap();
    menu.items
        .iter()
        .filter_map(|item| match item {
            gpui::MenuItem::Action { name, action, .. } => {
                Some((name.to_string(), action.partial_eq(&commands::Unavailable)))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_menu_bar_greys_what_cannot_run_and_ticks_what_is_on() {
    let states = MenuStates {
        unavailable: ["edit.undo".to_owned()].into(),
        checked: ["view.safeZones".to_owned(), "playback.goToEnd".to_owned()].into(),
    };
    let menus = menus_with_states("kerf", &[], None, &states);
    let edit = menu_items(&menus, "Edit");
    assert!(edit.contains(&("Undo".to_owned(), true)));
    assert!(edit.contains(&("Redo".to_owned(), false)));
    let view = menu_items(&menus, "View");
    assert!(view.contains(&("✓ Safe Zones".to_owned(), false)));
    // Only what can be ticked is.
    assert!(menu_items(&menus, "Playback").contains(&("Go to End".to_owned(), false)));
    // The catalog keeps which can be.
    let catalog = commands::catalog("kerf");
    let checkable = |id: &str| {
        catalog
            .iter()
            .find(|command| command.id == id)
            .unwrap()
            .checkable
    };
    assert!(checkable("view.safeZones") && checkable("tool.razor") && checkable("window.project"));
    assert!(!checkable("edit.undo"));
}

// ------------------------------------------------------------ the guide

#[test]
fn the_user_guide_opens_from_a_folder_with_spaces_and_only_with_something_in_it() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("Industrial Kerf.app").join("Contents");
    let exe = app.join("MacOS");
    let resources = app.join("Resources");
    std::fs::create_dir_all(&exe).unwrap();
    std::fs::create_dir_all(&resources).unwrap();
    let guide = resources.join("user-guide.html");
    std::fs::write(&guide, "").unwrap();
    // An empty guide does not count.
    assert_eq!(user_guide_in(&exe), None);
    std::fs::write(&guide, "<h1>Guide</h1>").unwrap();
    let url = user_guide_in(&exe).unwrap();
    assert!(url.starts_with("file:///"), "{url}");
    assert!(url.contains("Industrial%20Kerf.app"), "{url}");
    assert!(
        url.ends_with("/Contents/Resources/user-guide.html"),
        "{url}"
    );
    assert!(!url.contains(' '));
}

proptest! {
    #[test]
    fn a_file_url_reads_back_as_its_path(name in "[a-zA-Z0-9 _#%&()+,;=@~-]{1,24}") {
        let path = std::path::PathBuf::from(format!("/tmp/{name}/guide.html"));
        let url = file_url(&path);
        prop_assert!(!url.contains(' ') && !url.contains('#'));
        let decoded = {
            let bytes = url.trim_start_matches("file://").as_bytes();
            let mut out = Vec::new();
            let mut at = 0;
            while at < bytes.len() {
                if bytes[at] == b'%' {
                    let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap();
                    out.push(u8::from_str_radix(hex, 16).unwrap());
                    at += 3;
                } else {
                    out.push(bytes[at]);
                    at += 1;
                }
            }
            String::from_utf8(out).unwrap()
        };
        prop_assert_eq!(decoded, path.display().to_string());
    }
}

// ------------------------------------------------------------ the title

#[test]
fn the_title_says_when_it_was_edited_and_at_which_revision() {
    // Days are on a calendar: this one keeps to UTC's, whatever the
    // machine's zone (edited_ago itself counts the system's).
    let edited_ago = |then, now| edited_ago_in(then, now, &jiff::tz::TimeZone::UTC);
    let now = 1_800_000_000;
    assert_eq!(edited_ago(now - 20, now), "just now");
    assert_eq!(edited_ago(now - 120, now), "2 min ago");
    // Hours the same day; the day before is yesterday, however near.
    let day = now - now.rem_euclid(86_400);
    assert_eq!(edited_ago(day + 3600, day + 8 * 3600), "7 h ago");
    assert_eq!(edited_ago(day - 3600, day + 2 * 3600), "yesterday");
    assert_eq!(edited_ago(now - 3 * 86_400, now), "3 days ago");
    assert!(edited_ago(now - 40 * 86_400, now).ends_with("2026"));
    assert_eq!(
        title_tip("Atlas · Trailer", true, 22, Some(now - 120), now),
        "Atlas · Trailer · Edited 2 min ago · rev 22"
    );
    assert_eq!(title_tip("Atlas", false, 3, None, now), "rev 3");
}

#[test]
fn the_toolbar_says_what_renders_queued_exports_too() {
    let job = |status: &str, quality: &str, name: &str, progress: f64| RenderJob {
        job_id: format!("{status}-{quality}"),
        name: name.into(),
        quality: quality.into(),
        status: status.into(),
        progress,
        ..RenderJob::default()
    };
    let idle = Preview::default();
    assert_eq!(running_words(&[], &idle), None);
    assert_eq!(
        running_words(&[job("queued", "review", "", 0.0)], &idle).as_deref(),
        Some("Draft export · 0%")
    );
    // A running one before a queued one, whichever came first.
    assert_eq!(
        running_words(
            &[
                job("running", "master", "", 0.425),
                job("queued", "review", "", 0.0)
            ],
            &idle
        )
        .as_deref(),
        Some("Master export · 42%")
    );
    assert_eq!(
        running_words(&[job("completed", "master", "Master", 1.0)], &idle),
        None
    );
    let preview = Preview {
        status: "running".into(),
        progress: Some(0.5),
        ..Preview::default()
    };
    assert_eq!(
        running_words(&[], &preview).as_deref(),
        Some("Rendering preview · 50%")
    );
}

#[test]
fn the_sync_asks_for_the_words_it_needs() {
    let commit = sync_question("collab.commit", "main", "Harbour").unwrap();
    assert_eq!(commit.title, "Commit");
    assert_eq!(
        sync_params("collab.commit", " Trim the sea "),
        json!({"message": "Trim the sea"})
    );
    let request = sync_question("collab.pullRequest", "sea-trim_v2", "Harbour").unwrap();
    assert_eq!(request.text, "Sea trim v2");
    assert!(request.detail.contains("sea-trim_v2"));
    let publish = sync_question("collab.publish", "", "Harbour test").unwrap();
    assert_eq!(publish.text, "Harbour-test");
    assert_eq!(
        sync_params("collab.publish", "Harbour-test"),
        json!({"github": {"name": "Harbour-test", "private": true}})
    );
    assert!(sync_question("collab.sync", "", "").is_none());
}

// ------------------------------------------------------------ Focus Region

fn clip(json: Value) -> Clip {
    serde_json::from_value(json).unwrap()
}

proptest! {
    #[test]
    fn a_box_dragged_is_inside_the_frame_either_way_round(
        a in (-0.5f64..1.5, -0.5f64..1.5),
        b in (-0.5f64..1.5, -0.5f64..1.5),
    ) {
        let region = Region::between(a, b);
        prop_assert_eq!(region, Region::between(b, a));
        prop_assert!(region.x >= 0.0 && region.y >= 0.0);
        prop_assert!(region.x + region.width <= 1.0 + 1e-12);
        prop_assert!(region.y + region.height <= 1.0 + 1e-12);
    }

    #[test]
    fn a_crop_reads_back_as_its_box(
        x in 0.0f64..0.9, y in 0.0f64..0.9, w in 0.05f64..1.0, h in 0.05f64..1.0,
    ) {
        let region = Region { x, y, width: w.min(1.0 - x), height: h.min(1.0 - y) };
        let back = crop_box(&crop_props(Some(region))).unwrap();
        prop_assert!((back.x - region.x).abs() < 1e-9 && (back.y - region.y).abs() < 1e-9);
        prop_assert!((back.width - region.width).abs() < 1e-9);
        prop_assert!((back.height - region.height).abs() < 1e-9);
    }
}

#[test]
fn no_crop_is_cleared_and_reads_as_none() {
    let cleared = crop_props(None);
    assert!(cleared["crop_l"].is_null() && cleared["crop_b"].is_null());
    assert_eq!(crop_box(&cleared), None);
    assert_eq!(crop_box(&json!({})), None);
}

#[test]
fn focusing_splits_the_clip_round_the_range_and_crops_the_middle() {
    let region = Region {
        x: 0.25,
        y: 0.25,
        width: 0.5,
        height: 0.5,
    };
    let old = Some(Region {
        x: 0.1,
        y: 0.0,
        width: 0.8,
        height: 1.0,
    });
    let (ops, focused) = focus_ops("one", (0, 96), (24, 72), region, &[], old, "7");
    assert_eq!(focused, "one-focus-7");
    assert_eq!(
        ops[0],
        json!({"op": "split", "clip": "one", "at": 24, "tail": "one-focus-7"})
    );
    assert_eq!(ops[1]["op"], "set");
    assert_eq!(ops[1]["clip"], "one-focus-7");
    assert_eq!(ops[1]["props"]["crop_l"], 0.25);
    assert_eq!(
        ops[2],
        json!({"op": "split", "clip": "one-focus-7", "at": 72, "tail": "one-after-7"})
    );
    // After the range, the crop it had.
    assert_eq!(ops[3]["clip"], "one-after-7");
    assert_eq!(ops[3]["props"]["crop_l"], 0.1);
    // The whole clip: no split, the clip itself cropped.
    let (ops, focused) = focus_ops("one", (0, 96), (0, 96), region, &[], None, "7");
    assert_eq!(focused, "one");
    assert_eq!(ops.len(), 1);
}

#[test]
fn a_followed_subject_is_a_part_a_step() {
    let region = Region {
        x: 0.2,
        y: 0.2,
        width: 0.3,
        height: 0.3,
    };
    let moved = Region { x: 0.3, ..region };
    let tracked = [(0, region), (24, moved), (200, moved)];
    let (ops, focused) = focus_ops("one", (0, 96), (0, 96), region, &tracked, None, "s");
    // The step past the range is not split off.
    assert_eq!(focused, "one-track-1-s");
    assert_eq!(ops.len(), 3);
    assert_eq!(
        ops[1],
        json!({"op": "split", "clip": "one", "at": 24, "tail": "one-track-1-s"})
    );
    assert_eq!(ops[2]["props"]["crop_l"], 0.3);
}

#[test]
fn a_scene_cut_inside_the_range_is_where_the_box_ends() {
    // Media seconds 0..4 play at 2..6 of the program.
    let clip =
        clip(json!({"id": "one", "timelineStart": 2.0, "duration": 4.0, "in": 0.0, "out": 4.0}));
    assert_eq!(first_cut(&[1.5, 3.0], &clip, 2.0, 6.0, 24.0), Some(3.5));
    // Outside the range, or on its ends, none.
    assert_eq!(first_cut(&[0.0, 3.99], &clip, 2.0, 6.0, 24.0), None);
    assert_eq!(first_cut(&[], &clip, 2.0, 6.0, 24.0), None);
    // A clip played twice as fast reaches its cuts sooner.
    let fast = clip_twice();
    assert_eq!(first_cut(&[4.0], &fast, 0.0, 4.0, 24.0), Some(2.0));
}

fn clip_twice() -> Clip {
    clip(
        json!({"id": "fast", "timelineStart": 0.0, "duration": 4.0, "in": 0.0, "out": 8.0, "speed": 2.0}),
    )
}

#[test]
fn times_are_typed_as_seconds_frames_or_timecode() {
    assert_eq!(typed_time("1.500 s", 24.0), Some(1.5));
    assert_eq!(typed_time("2", 24.0), Some(2.0));
    assert_eq!(typed_time("48f", 24.0), Some(2.0));
    assert_eq!(typed_time("00:00:01:12", 24.0), Some(1.5));
    assert_eq!(typed_time("soon", 24.0), None);
    assert_eq!(typed_time(&shown_time(3.25), 24.0), Some(3.25));
}

/// Grey frames of a bright square on a dark ground, at `(x, y)` in each.
fn frames_of(places: &[(usize, usize)]) -> Vec<u8> {
    let (width, height) = TRACK_SIZE;
    let mut frames = Vec::new();
    for (x, y) in places {
        let mut frame = vec![20u8; width * height];
        for row in 0..20 {
            for column in 0..20 {
                // A pattern inside, so the square is not flat.
                frame[(y + row) * width + x + column] = if (row / 4 + column / 4) % 2 == 0 {
                    230
                } else {
                    120
                };
            }
        }
        frames.extend(frame);
    }
    frames
}

#[test]
fn a_subject_that_moves_is_followed_and_a_flat_box_is_not() {
    let (width, height) = TRACK_SIZE;
    let seed = Region {
        x: 40.0 / width as f64,
        y: 30.0 / height as f64,
        width: 20.0 / width as f64,
        height: 20.0 / height as f64,
    };
    let frames = frames_of(&[(40, 30), (44, 31), (49, 33), (55, 34)]);
    let tracked = track_region(&frames, TRACK_SIZE, seed, (0.0, 10.0), 24.0).unwrap();
    assert_eq!(tracked.reliable_end, 10.0);
    // The start, then each frame it moved in (two a second: 12 frames apart).
    let frames_at: Vec<i64> = tracked.boxes.iter().map(|(frame, _)| *frame).collect();
    assert_eq!(frames_at, [0, 12, 24, 36]);
    let last = tracked.boxes.last().unwrap().1;
    assert!((last.x * width as f64 - 55.0).abs() < 1.0, "{last:?}");
    assert!((last.y * height as f64 - 34.0).abs() < 1.0, "{last:?}");
    // A box with nothing in it cannot be followed; nor one frame.
    let flat = Region {
        x: 0.7,
        y: 0.7,
        width: 0.2,
        height: 0.2,
    };
    assert_eq!(
        track_region(&frames, TRACK_SIZE, flat, (0.0, 10.0), 24.0),
        None
    );
    assert_eq!(
        track_region(&frames_of(&[(40, 30)]), TRACK_SIZE, seed, (0.0, 10.0), 24.0),
        None
    );
    // Gone, it is lost where it went.
    let mut gone = frames_of(&[(40, 30), (42, 30)]);
    gone.extend(vec![20u8; width * height]);
    let lost = track_region(&gone, TRACK_SIZE, seed, (0.0, 10.0), 24.0).unwrap();
    assert_eq!(lost.reliable_end, 1.0);
}

// ------------------------------------------------------------ the picture

#[test]
fn the_transport_runs_from_start_to_end_and_says_why_it_cannot() {
    let ids: Vec<&str> = TRANSPORT.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(
        ids,
        [
            "playback.goToStart",
            "playback.previousFrame",
            "playback.toggle",
            "playback.nextFrame",
            "playback.goToEnd"
        ]
    );
    assert_eq!(transport_tip("Go to start", None), "Go to start");
    assert_eq!(
        transport_tip("Go to end", Some("Not connected")),
        "Go to end · Not connected"
    );
}

#[test]
fn the_chip_says_cutting_only_while_kerf_builds_a_first_cut() {
    assert!(kerf_cutting(true, false, true));
    // Looking at clips there already, or with nothing proposed: looking.
    assert!(!kerf_cutting(true, true, true));
    assert!(!kerf_cutting(true, false, false));
    assert!(!kerf_cutting(false, false, true));
}

#[test]
fn the_working_dot_pulses_down_and_back() {
    assert!((pulse_level(0.0) - 1.0).abs() < 1e-6);
    assert!((pulse_level(1.0) - 1.0).abs() < 1e-6);
    assert!((pulse_level(0.5) - 0.35).abs() < 1e-6);
    for step in 0..=24 {
        let level = pulse_level(step as f32 / 24.0);
        assert!((0.35 - 1e-6..=1.0 + 1e-6).contains(&level));
    }
}

fn timeline(value: Value) -> Timeline {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_poster_stands_in_for_the_topmost_footage_at_the_playhead() {
    let cut = timeline(json!({"tracks": [
        {"id": "V1", "type": "video", "clips": [
            {"id": "one", "timelineStart": 0.0, "duration": 4.0, "sourceId": "a"},
            {"id": "two", "timelineStart": 4.0, "duration": 3.0, "sourceId": "b"}]},
        {"id": "V2", "type": "video", "clips": [
            {"id": "title", "timelineStart": 1.0, "duration": 2.0},
            {"id": "over", "timelineStart": 5.0, "duration": 1.0, "sourceId": "c"}]},
        {"id": "A1", "type": "audio", "clips": [
            {"id": "sound", "timelineStart": 0.0, "duration": 7.0, "sourceId": "s"}]}
    ]}));
    // A title has no media of its own: the footage under it.
    assert_eq!(poster_source(&cut, 1.5).as_deref(), Some("a"));
    assert_eq!(poster_source(&cut, 5.5).as_deref(), Some("c"));
    assert_eq!(poster_source(&cut, 4.5).as_deref(), Some("b"));
    // Past the end, the first footage.
    assert_eq!(poster_source(&cut, 30.0).as_deref(), Some("a"));
    assert_eq!(poster_source(&timeline(json!({"tracks": []})), 0.0), None);
}

#[test]
fn a_cut_built_shows_each_clip_as_it_is_placed() {
    let cut = timeline(json!({"tracks": [
        {"id": "V1", "type": "video", "clips": [
            {"id": "late", "timelineStart": 3.0, "duration": 1.0, "sourceId": "b"},
            {"id": "early", "timelineStart": 0.0, "duration": 3.0, "sourceId": "a"}]}
    ]}));
    let mut seen = HashSet::new();
    assert_eq!(
        placed_clips(&seen, &cut),
        [
            ("early".to_owned(), "a".to_owned()),
            ("late".to_owned(), "b".to_owned())
        ]
    );
    seen.insert("early".to_owned());
    assert_eq!(
        placed_clips(&seen, &cut),
        [("late".to_owned(), "b".to_owned())]
    );
}

#[test]
fn going_home_travels_whole_then_fades_where_it_landed() {
    let (start, clear) = home_flight_at(Duration::ZERO).unwrap();
    assert_eq!((start, clear), (0.0, 1.0));
    let stage = kerf_editor::theme::motion::STAGE;
    let land = kerf_editor::theme::motion::FLIGHT_LAND;
    let (there, clear) = home_flight_at(stage).unwrap();
    assert!((there - 1.0).abs() < 1e-6 && (clear - 1.0).abs() < 1e-6);
    let (_, fading) = home_flight_at(stage + land / 2).unwrap();
    assert!(fading > 0.0 && fading < 1.0);
    assert_eq!(home_flight_at(stage + land), None);
}

#[test]
fn the_compare_line_sweeps_there_and_back_and_holds_still_without_motion() {
    let from = compare_at(Duration::ZERO, false);
    let half = compare_at(Duration::from_millis(3400), false);
    let round = compare_at(Duration::from_millis(6800), false);
    assert!((from - 0.28).abs() < 1e-4);
    assert!((half - 0.78).abs() < 1e-4);
    assert!((round - 0.28).abs() < 1e-4);
    for ms in (0..13_600).step_by(170) {
        let at = compare_at(Duration::from_millis(ms), false);
        assert!((0.28 - 1e-4..=0.78 + 1e-4).contains(&at));
    }
    assert_eq!(compare_at(Duration::from_millis(1234), true), 0.28);
}

proptest! {
    #[test]
    fn base64_reads_back_what_was_written(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut text = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (at, byte)| n | u32::from(*byte) << (16 - 8 * at));
            for at in 0..4 {
                if at <= chunk.len() {
                    text.push(TABLE[(n >> (18 - 6 * at) & 63) as usize] as char);
                } else {
                    text.push('=');
                }
            }
        }
        prop_assert_eq!(base64_decode(&text), Some(bytes));
    }
}

#[test]
fn base64_that_does_not_read_is_none() {
    assert_eq!(base64_decode("ab$d"), None);
    assert_eq!(base64_decode("abcde"), None);
    assert_eq!(base64_decode("TWFu").as_deref(), Some(&b"Man"[..]));
}

#[test]
fn the_light_over_a_working_title_moves_in_steps() {
    let steps = sidebar::TITLE_SHIMMER_STEPS as usize;
    assert_eq!(sidebar::shimmer_step(0.0), 0.0);
    assert_eq!(
        sidebar::shimmer_step(0.001),
        sidebar::shimmer_step(1.0 / steps as f32 - 0.001)
    );
    assert!(sidebar::shimmer_step(1.0) < 1.0);
    let mut seen: Vec<f32> = (0..10_000)
        .map(|at| sidebar::shimmer_step(at as f32 / 10_000.0))
        .collect();
    seen.dedup();
    assert_eq!(seen.len(), steps);
}

// ------------------------------------------------------------ the app's menu in a short window

#[test]
fn the_app_menu_stays_inside_a_short_window_and_scrolls_when_it_cannot_fit() {
    // Over the account, from just under the window's top to just over the
    // account; under the ⋯, from the toolbar's foot to just over the
    // window's.
    assert_eq!(app_menu_room(true, 1000.0), (4.0, 1000.0 - 4.0 - 48.0));
    assert_eq!(app_menu_room(false, 1000.0), (38.0, 1000.0 - 38.0 - 4.0));
    // A menu that fits stands on the account, or hangs from the ⋯.
    let (_, room) = app_menu_room(true, 1000.0);
    assert_eq!(app_menu_place(true, room, 520.0), (room - 520.0, false));
    assert_eq!(app_menu_place(false, room, 520.0), (0.0, false));
    // In the shortest window it cannot: it starts at the room's top and
    // scrolls there, from the account or the ⋯.
    for above_account in [true, false] {
        let (_, room) = app_menu_room(above_account, 480.0);
        assert!(room < 520.0);
        assert_eq!(app_menu_place(above_account, room, 520.0), (0.0, true));
    }
}

proptest! {
    #[test]
    fn the_app_menu_never_starts_outside_its_room(
        above_account in any::<bool>(),
        window in 0.0f32..2000.0,
        tall in 0.0f32..1200.0,
    ) {
        let (top, room) = app_menu_room(above_account, window);
        prop_assert!(top >= 0.0 && room >= 0.0);
        prop_assert!(top + room <= window.max(top));
        let (down, scrolls) = app_menu_place(above_account, room, tall);
        prop_assert!(down >= 0.0);
        prop_assert_eq!(scrolls, tall > room);
        if !scrolls {
            // All of it is in the room, every row in reach.
            prop_assert!(down + tall <= room + 1e-3);
        }
    }
}

#[test]
fn switch_branch_in_the_app_menu_offers_the_other_branches() {
    let commands = commands::catalog("kerf");
    let menus = vec!["Collaborate".to_owned()];
    let status = json!({
        "repository": true, "branch": "main", "branches": ["main", "trailer", "colour"],
        "behind": 0, "ahead": 0, "conflicted": false, "live": false,
    });
    let states = json!({});
    let checked = BTreeSet::new();
    let rows = app_rows(&input(
        &commands,
        &menus,
        &states,
        &checked,
        Some(&status),
        false,
    ));
    let switch = find(&find(&rows, "Collaborate").rows, "Switch Branch");
    // It asks for nothing itself: its rows are the branches it goes to.
    assert_eq!(switch.does, AppDoes::Nothing);
    let to: Vec<&AppDoes> = switch.rows.iter().map(|row| &row.does).collect();
    assert_eq!(
        to,
        [
            &AppDoes::Branch("trailer".into()),
            &AppDoes::Branch("colour".into())
        ]
    );
    // With none to go to it says so; and it is greyed with why while the
    // registry says it cannot run.
    let alone = json!({"repository": true, "branch": "main", "branches": ["main"]});
    let states = json!({"collab.switchBranch": "There are no other branches"});
    let rows = app_rows(&input(
        &commands,
        &menus,
        &states,
        &checked,
        Some(&alone),
        false,
    ));
    let switch = find(&find(&rows, "Collaborate").rows, "Switch Branch");
    assert!(switch.disabled);
    assert_eq!(switch.tip.as_deref(), Some("There are no other branches"));
    assert_eq!(switch.rows.len(), 1);
    assert_eq!(switch.rows[0].words, "No Other Branches");
    assert!(switch.rows[0].disabled);
}

// ------------------------------------------------------------ Focus Region's frame

#[test]
fn focus_region_needs_a_frame_of_the_clips_media() {
    let footage =
        clip(json!({"id": "a", "timelineStart": 10.0, "duration": 4.0, "in": 2.0, "out": 6.0}));
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("a.mov");
    std::fs::write(&file, b"not much").unwrap();
    // The frame at program second 11 is the media's second 3.
    let (read, seconds) = readable_frame(Some(file.clone()), &footage, 11.0).unwrap();
    assert_eq!(read, file);
    assert!((seconds - 3.0).abs() < 1e-9);
    // A title or a solid has no media; a file gone cannot be read.
    assert_eq!(readable_frame(None, &footage, 11.0), Err(NO_FRAME));
    assert_eq!(
        readable_frame(Some(folder.path().join("gone.mov")), &footage, 11.0),
        Err(NO_FRAME)
    );
    let title = clip(json!({"id": "t", "timelineStart": 0.0, "duration": 2.0}));
    assert_eq!(readable_frame(Some(file), &title, 1.0), Err(NO_FRAME));
}

#[test]
fn ffmpeg_is_looked_for_where_kerfd_looks() {
    // The order it is looked for in is kerf_engine's (its own tests hold
    // it); the editor runs the very one kerfd finds.
    assert_eq!(
        ffmpeg_bin(),
        std::path::Path::new(kerf_bridge::ffmpeg_bin())
    );
}

// ------------------------------------------------------------ the picture standing in, the transport

#[test]
fn the_cuts_render_coming_releases_the_poster_standing_in() {
    let preview = |status: &str, proposal: &str| json!({"type": "preview", "status": status, "proposalId": proposal});
    assert!(!releases_cover(&preview("running", "p2"), Some("p2")));
    assert!(releases_cover(&preview("ready", "p2"), Some("p2")));
    assert!(releases_cover(&preview("failed", "p2"), Some("p2")));
    // A render of a version before it, or with no cut being built: not yet.
    assert!(!releases_cover(&preview("ready", "p1"), Some("p2")));
    assert!(!releases_cover(&preview("ready", "p2"), None));
    assert!(!releases_cover(
        &json!({"type": "render", "status": "complete", "proposalId": "p2"}),
        Some("p2")
    ));
}

#[test]
fn the_transport_says_play_waits_for_a_preview() {
    let gone = transport_reasons(&json!({"connected": false}));
    assert_eq!(gone.len(), TRANSPORT.len());
    let connected = transport_reasons(&json!({"connected": true}));
    assert!(connected.is_empty(), "{connected:?}");
    let required = transport_reasons(&json!({"connected": true, "previewRequired": true}));
    assert_eq!(required.len(), 1);
    assert_eq!(
        required.get("playback.toggle").map(String::as_str),
        Some("Render a preview to play this timeline")
    );
}
