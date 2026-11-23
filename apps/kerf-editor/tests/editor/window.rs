//! The project window: its layout, toolbar and chrome, dialogs, menus and tips, and several windows at once.

use crate::support::{bounds, rect, window};
use gpui::{point, px, size, Action, Bounds, MenuItem, WindowBounds};
use jiff::tz::TimeZone;
use kerf_editor::app::{
    crash_notice_text, parse_window_size, path_with_tool_folders, titlebar, window_buttons_at,
    window_place, x11_rgba, X11Pixels,
};
use kerf_editor::chrome::lead_past;
use kerf_editor::commands::{
    recent_rows, HideApp, HideOtherApps, MenuBranches, MenuStates, NoOtherBranches, RecentRow,
    ShowAllApps, Unavailable, NO_MASK_CLIP,
};
use kerf_editor::finish::NO_OTHER_BRANCHES;
use kerf_editor::keymap::{
    app_menu, app_menu_bindings, branch_menu, menus_with, menus_with_branches, menus_with_states,
    recent_menu,
};
use kerf_editor::model::{RenderJob, RenderQuality, RenderStatus};
use kerf_editor::prefs::Prefs;
use kerf_editor::settings::component_name;
use kerf_editor::sidebar::{
    app_rows, sync_problem, with_sync_failure, AppDoes, AppMenuInput, AppRow, SwitchBranch,
    SYNC_FAILURE,
};
use kerf_editor::stage::{mask_tool_id, MaskTool};
use kerf_editor::store::{Connection, Preview};
use kerf_editor::theme::{layout, text};
use kerf_editor::ui::{
    modal_bounds, tip_anchor, tip_for, tip_height, tip_is_quick, tip_origin, Tip,
};
use kerf_editor::workspace::{
    app_menu_place, app_menu_room, bar_takes_error, edited_ago_changes_in, edited_ago_in,
    first_line, frame_step, kept_in_bar, local_day, panel_heights, panes_opening, picture_in,
    picture_within, readable_error, running_words, standing_problem, sync_failed, sync_params,
    sync_question, timeline_split, title_tip, toolbar_places, StandingInput, ToolbarPlaces,
};
use kerf_editor::{commands, keymap, sidebar, theme};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ------------------------------------------------------------ tips

#[test]
fn a_greyed_control_says_why_and_keeps_its_key() {
    let name = "Run QC".into();
    let key = "⌘R".into();
    let reason = "Connect to check".into();
    assert_eq!(
        tip_for(Some(&name), Some(&key), false, Some(&reason)),
        Some(Tip::new("Run QC").key("⌘R"))
    );
    assert_eq!(
        tip_for(Some(&name), Some(&key), true, Some(&reason)),
        Some(Tip::new("Connect to check").key("⌘R"))
    );
    assert_eq!(
        tip_for(Some(&name), None, true, None),
        Some(Tip::new("Run QC"))
    );
    assert_eq!(tip_for(None, Some(&key), false, Some(&reason)), None);
}

#[test]
fn once_a_tip_has_shown_the_next_shows_at_once() {
    let now = Instant::now() + Duration::from_secs(10);
    assert!(!tip_is_quick(0, None, now));
    assert!(tip_is_quick(1, None, now));
    assert!(tip_is_quick(0, Some(now - Duration::from_millis(100)), now));
    assert!(!tip_is_quick(
        0,
        Some(now - theme::motion::TIP_QUICK - Duration::from_millis(1)),
        now
    ));
}

proptest! {
    /// A tip keeps inside the window's sides and never covers its control.
    #[test]
    fn a_tip_stands_by_its_control_inside_the_window(
        left in 0f32..1500.0,
        top in 0f32..950.0,
        width in 16f32..200.0,
        tip_width in 30f32..300.0,
    ) {
        let window = size(px(1600.0), px(1000.0));
        let control = Bounds::new(point(px(left), px(top)), size(px(width), px(28.0)));
        let tip = size(px(tip_width), px(layout::TIP_HEIGHT));
        let at = tip_origin(control, tip, window);
        let (x, y) = (f32::from(at.x), f32::from(at.y));
        prop_assert!(x >= layout::TIP_EDGE - 0.5);
        prop_assert!(x + tip_width <= 1600.0 - layout::TIP_EDGE + 0.5);
        let below = y >= top + 28.0;
        let above = y + layout::TIP_HEIGHT <= top;
        prop_assert!(below || above);
    }
}

#[test]
fn a_tip_goes_over_a_control_low_in_the_window() {
    let window = size(px(1600.0), px(1000.0));
    let tip = size(px(80.0), px(layout::TIP_HEIGHT));
    let toolbar = Bounds::new(point(px(400.0), px(6.0)), size(px(28.0), px(28.0)));
    let transport = Bounds::new(point(px(400.0), px(900.0)), size(px(28.0), px(28.0)));
    assert_eq!(
        tip_origin(toolbar, tip, window),
        point(px(374.0), px(6.0 + 28.0 + layout::TIP_GAP))
    );
    assert_eq!(
        tip_origin(transport, tip, window),
        point(px(374.0), px(900.0 - layout::TIP_GAP - layout::TIP_HEIGHT))
    );
}

proptest! {
    /// A panel is never bigger than it is made, and in a window with room
    /// for the margins it keeps them, centred, with a quarter of the room
    /// to spare above it.
    #[test]
    fn a_panel_fits_the_window(
        most_w in 200f32..1200.0, most_h in 200f32..900.0,
        window_w in 300f32..2400.0, window_h in 300f32..1600.0,
    ) {
        let panel = modal_bounds(size(px(most_w), px(most_h)), size(px(window_w), px(window_h)));
        let (x, y) = (f32::from(panel.origin.x), f32::from(panel.origin.y));
        let (w, h) = (f32::from(panel.size.width), f32::from(panel.size.height));
        prop_assert!(w <= most_w && h <= most_h);
        prop_assert!(w <= window_w - 2.0 * layout::MODAL_MARGIN + 0.01);
        prop_assert!(h <= window_h - 2.0 * layout::MODAL_MARGIN + 0.01);
        prop_assert!(x >= layout::MODAL_MARGIN - 1.0 || w >= most_w - 0.01);
        prop_assert!((x - (window_w - w) / 2.0).abs() <= 1.0);
        prop_assert!((y - (window_h - h) * layout::MODAL_ABOVE).abs() <= 1.0);
    }
}

const DISCONNECTED: &str = "Runtime is disconnected; reconnect to continue";

// ---------------------------------------------------------------- the toolbar's red line

fn standing_input<'a>(
    connection: &'a Connection,
    error: Option<&'a str>,
    conflicted: bool,
    sync_failure: Option<&'a str>,
) -> StandingInput<'a> {
    StandingInput {
        has_project: true,
        connection,
        error,
        conflicted,
        sync_failure,
    }
}

#[test]
fn the_red_line_says_a_failed_sync_after_its_bar_is_dismissed() {
    let connected = Connection::Connected;
    // The bar shows: the line says its first line.
    let bar = sync_failed("could not reach origin");
    assert_eq!(bar, "Sync failed: could not reach origin");
    assert_eq!(
        standing_problem(&standing_input(
            &connected,
            Some(&bar),
            false,
            Some("could not reach origin")
        )),
        Some("Sync failed: could not reach origin".into())
    );
    // Dismissed: the line still says it, until a later sync starts.
    assert_eq!(
        standing_problem(&standing_input(
            &connected,
            None,
            false,
            Some("could not reach origin")
        )),
        Some("Sync failed: could not reach origin".into())
    );
    assert_eq!(
        standing_problem(&standing_input(&connected, None, false, None)),
        None
    );
}

#[test]
fn the_red_line_keeps_its_order() {
    let connected = Connection::Connected;
    let failed = Connection::Failed("gone".into());
    let connecting = Connection::Connecting;
    let why = Some("no route");
    assert_eq!(
        standing_problem(&standing_input(&failed, Some("an error"), true, why)),
        Some("Offline".into())
    );
    assert_eq!(
        standing_problem(&standing_input(&connecting, Some("an error"), true, why)),
        Some("Connecting…".into())
    );
    assert_eq!(
        standing_problem(&standing_input(
            &connected,
            Some("an error\nmore"),
            true,
            why
        )),
        Some("an error".into())
    );
    assert_eq!(
        standing_problem(&standing_input(&connected, None, true, why)),
        Some("A merge is unfinished; finish it with git".into())
    );
    assert_eq!(
        standing_problem(&standing_input(&connected, None, false, why)),
        Some("Sync failed: no route".into())
    );
    // No project: nothing stands in the way of one.
    let mut none = standing_input(&failed, Some("an error"), true, why);
    none.has_project = false;
    assert_eq!(standing_problem(&none), None);
}

#[test]
fn a_failed_sync_reads_as_a_person_does() {
    assert_eq!(
        sync_failed("error[git.fetch]: could not read from remote"),
        "Sync failed: could not read from remote"
    );
    // A long failure (git's lines) is one line in the toolbar.
    let connected = Connection::Connected;
    assert_eq!(
        standing_problem(&standing_input(
            &connected,
            None,
            false,
            Some("fatal: '/gone' does not appear to be a git repository\nfatal: Could not read")
        )),
        Some("Sync failed: fatal: '/gone' does not appear to be a git repository".into())
    );
}

// ---------------------------------------------------------------- disconnected

#[test]
fn runtime_is_disconnected_has_no_bar_while_kerfd_is_away() {
    assert!(!bar_takes_error(
        DISCONNECTED,
        &Connection::Failed("gone".into())
    ));
    assert!(!bar_takes_error(DISCONNECTED, &Connection::Connecting));
    // Reached again meanwhile: it is said.
    assert!(bar_takes_error(DISCONNECTED, &Connection::Connected));
    // Any other error has its bar whatever the connection.
    assert!(bar_takes_error(
        "Clip not found",
        &Connection::Failed("gone".into())
    ));
    assert!(bar_takes_error("Clip not found", &Connection::Connecting));
}

proptest! {

    #[test]
    fn the_panel_is_shared_out_whole(
        height in 0.0f32..2000.0,
        width in 1.0f32..3000.0,
        aspect in 0.3f32..3.0,
        picture in 0.0f32..=1.0,
        timeline in 0.0f32..=1.0,
        media in 0.0f32..=1.0,
    ) {
        let (picture_tall, band, lanes) =
            panel_heights(height, width, aspect, (picture, timeline, media));
        prop_assert!(picture_tall >= 0.0 && band >= 0.0 && lanes >= 0.0);
        prop_assert!(picture_tall + band + lanes <= height.max(band) + 0.5);
        if timeline == 0.0 {
            prop_assert_eq!(lanes, 0.0);
        }
    }

    #[test]
    fn a_picture_placed_in_its_slot_keeps_its_shape(
        width in 200.0f32..2000.0, height in 200.0f32..1500.0, aspect in 0.4f32..2.5,
    ) {
        let slot = rect(100.0, 50.0, width, height);
        let placed = picture_in(slot, aspect);
        let shape = f32::from(placed.size.width) / f32::from(placed.size.height);
        prop_assert!((shape - aspect).abs() / aspect < 0.01);
        prop_assert!(placed.left() >= slot.left() && placed.right() <= slot.right() + px(0.5));
        prop_assert!(placed.top() >= slot.top() && placed.bottom() <= slot.bottom());
    }
}

#[test]
fn a_frame_without_a_size_is_placed_as_a_wide_picture() {
    let slot = rect(0.0, 0.0, 1000.0, 700.0);
    assert_eq!(picture_within(slot, 0, 0), picture_in(slot, 16.0 / 9.0));
    assert_eq!(
        picture_within(slot, 1080, 1920),
        picture_in(slot, 1080.0 / 1920.0)
    );
}

#[test]
fn an_error_bar_says_the_first_line_of_a_long_error() {
    let refused = "cannot import: ffmpeg failed (1): ffprobe exited with code 1\n\
                   args: -v quiet -print_format json -show_format /tmp/notes.mov\n\
                   stderr:";
    assert_eq!(
        first_line(refused),
        "cannot import: ffmpeg failed (1): ffprobe exited with code 1"
    );
    assert_eq!(first_line("\n  \n  Offline  \nmore"), "Offline");
    assert_eq!(first_line("One line"), "One line");
    assert_eq!(first_line(""), "");
}

proptest! {
    #[test]
    fn the_first_line_is_one_line_of_what_was_said(text in "[a-z \\n]{0,80}") {
        let line = first_line(&text);
        prop_assert!(!line.contains('\n'));
        prop_assert!(text.contains(line));
        prop_assert_eq!(line.is_empty(), text.trim().is_empty());
    }

    #[test]
    fn a_frame_step_lands_on_the_frame_grid(
        position in 0.0f64..600.0,
        rate in prop::sample::select(vec![23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 60.0]),
        forward in any::<bool>(),
    ) {
        let frames = if forward { 1 } else { -1 };
        let at = frame_step(position, rate, frames);
        prop_assert!(at >= 0.0);
        let frame = at * rate;
        prop_assert!((frame - frame.round()).abs() < 1e-6, "{at} is not on a frame at {rate}");
        // One frame from the playhead's own frame, the start holding.
        let own = (position * rate).round();
        prop_assert!((frame - (own + frames as f64).max(0.0)).abs() < 1e-6);
        // And back again is that frame.
        if (own + frames as f64) >= 0.0 {
            let back = frame_step(at, rate, -frames);
            prop_assert!((back * rate - own).abs() < 1e-6);
        }
    }

    #[test]
    fn the_panel_splits_by_the_pictures_shape(
        height in 200f32..2000.0,
        width in 200f32..2400.0,
        aspect in 0.4f32..3.0,
        band in prop::sample::select(vec![0.0f32, layout::MEDIA_BAND]),
    ) {
        let timeline = timeline_split(height, band, width, aspect);
        let picture = height - band - timeline;
        prop_assert!(timeline >= 0.0);
        prop_assert!(picture >= -0.01, "the picture is never less than nothing");
        // The transport is always under the picture.
        prop_assert!(picture >= layout::TRANSPORT.min(height - band) - 0.01);
        if aspect >= layout::WIDE {
            // A wide picture never has more height than its width uses:
            // the timeline has the rest.
            let air = 64.0;
            prop_assert!(picture - layout::TRANSPORT <= (width - air).max(0.0) / aspect + air + 0.01);
        }
        if band > 0.0 && height - band >= layout::MEDIA_BAND_TIMELINE_LEAST + layout::TRANSPORT {
            // The band leaves the timeline (with the transport) its least.
            prop_assert!(timeline + layout::TRANSPORT >= layout::MEDIA_BAND_TIMELINE_LEAST - 0.01);
        }
    }
}

#[test]
fn a_vertical_picture_leaves_the_timeline_less_than_a_wide_one() {
    // The panel of a 1600 × 1000 window with the sidebar out.
    let (height, width) = (952.0, 864.0);
    let wide = timeline_split(height, 0.0, width, 16.0 / 9.0);
    let vertical = timeline_split(height, 0.0, width, 9.0 / 16.0);
    // Under a wide picture 56% of the height, the transport counted in.
    let wide_share = height * layout::TIMELINE_WIDE_SHARE;
    assert!(
        (wide + layout::TRANSPORT - wide_share).abs() < 0.5,
        "{wide}"
    );
    let upright_share = height * layout::TIMELINE_UPRIGHT_SHARE;
    assert!(
        (vertical + layout::TRANSPORT - upright_share).abs() < 0.5,
        "{vertical}"
    );
    assert!(vertical < wide);
    // A wide picture in a narrow stage leaves the timeline what it cannot use.
    let narrow = timeline_split(height, 0.0, 400.0, 16.0 / 9.0);
    assert!(narrow > wide + 100.0, "{narrow}");
}

#[test]
fn the_view_menu_has_the_mask_tools() {
    let menus = menus_with("kerf", &[], None);
    let view = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "View")
        .expect("a View menu");
    let names: Vec<&str> = view
        .items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Action { name, .. } => Some(name.as_ref()),
            _ => None,
        })
        .collect();
    let tools = &names[names.len() - 3..];
    assert_eq!(tools, ["Rectangle mask", "Ellipse mask", "Pen mask"]);
    // A separator before them, after the registry's own.
    let at = view.items.len() - 4;
    assert!(matches!(view.items[at], MenuItem::Separator));
}

// ------------------------------------------------------------ the menu bar

fn action_names(items: &[MenuItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Action { name, .. } => Some(name.to_string()),
            MenuItem::SystemMenu(menu) => Some(format!("{} ▸", menu.name)),
            MenuItem::Submenu(menu) => Some(format!("{} ▸", menu.name)),
            MenuItem::Separator => None,
        })
        .collect()
}

#[test]
fn the_application_menu_has_services_hide_hide_others_and_show_all() {
    let menu = app_menu();
    assert_eq!(menu.name.as_ref(), "Industrial Kerf");
    assert_eq!(
        action_names(&menu.items),
        [
            "About Kerf",
            "Settings…",
            "Services ▸",
            "Hide Industrial Kerf",
            "Hide Others",
            "Show All",
            "Quit Industrial Kerf",
        ]
    );
    // Services is the system's own.
    assert!(menu.items.iter().any(|item| matches!(
        item,
        MenuItem::SystemMenu(menu) if matches!(menu.menu_type, gpui::SystemMenuType::Services)
    )));
    let action_of = |words: &str| {
        menu.items
            .iter()
            .find_map(|item| match item {
                MenuItem::Action { name, action, .. } if name == words => {
                    Some(action.boxed_clone())
                }
                _ => None,
            })
            .unwrap()
    };
    assert!(action_of("Hide Industrial Kerf").partial_eq(&HideApp));
    assert!(action_of("Hide Others").partial_eq(&HideOtherApps));
    assert!(action_of("Show All").partial_eq(&ShowAllApps));
    // The menu bar leads with it.
    let menus = keymap::menus("kerf");
    assert_eq!(action_names(&menus[0].items), action_names(&menu.items));
    // ⌘H and ⌥⌘H, on macOS only: elsewhere there is no such menu.
    let keys: Vec<String> = app_menu_bindings()
        .iter()
        .map(|binding| {
            binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.unparse())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    if cfg!(target_os = "macos") {
        assert_eq!(keys, ["cmd-h", "alt-cmd-h"]);
    } else {
        assert!(keys.is_empty(), "{keys:?}");
    }
}

/// View's mask tool rows in the menu bar: their names and whether each is
/// greyed (Unavailable).
fn mask_rows(states: &MenuStates) -> Vec<(String, bool)> {
    let menus = menus_with_states("kerf", &[], None, states);
    let view = menus.iter().find(|menu| menu.name == "View").unwrap();
    view.items
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(|item| match item {
            MenuItem::Action { name, action, .. } => {
                (name.to_string(), action.partial_eq(&Unavailable))
            }
            _ => panic!("a mask tool row"),
        })
        .collect()
}

#[test]
fn the_menu_bars_mask_tools_are_ticked_while_out_and_greyed_with_no_clip_to_draw_on() {
    let free = mask_rows(&MenuStates::default());
    assert_eq!(
        free,
        [
            ("Rectangle mask".to_owned(), false),
            ("Ellipse mask".to_owned(), false),
            ("Pen mask".to_owned(), false),
        ]
    );
    let none = MenuStates {
        unavailable: MaskTool::ALL
            .map(|tool| mask_tool_id(tool).to_owned())
            .into(),
        checked: BTreeSet::new(),
    };
    assert!(mask_rows(&none).iter().all(|(_, greyed)| *greyed));
    let pen = MenuStates {
        unavailable: BTreeSet::new(),
        checked: [mask_tool_id(MaskTool::Pen).to_owned()].into(),
    };
    assert_eq!(
        mask_rows(&pen),
        [
            ("Rectangle mask".to_owned(), false),
            ("Ellipse mask".to_owned(), false),
            ("✓ Pen mask".to_owned(), false),
        ]
    );
    // Each has an id of its own.
    let ids: BTreeSet<&str> = MaskTool::ALL.map(mask_tool_id).into();
    assert_eq!(ids.len(), 3);
}

// ------------------------------------------------------------ the app's menu

fn kerf_menu_input<'a>(
    commands: &'a [commands::Command],
    menus: &'a [String],
    states: &'a Value,
    checked: &'a BTreeSet<String>,
    collab: Option<&'a Value>,
    menu_bar: bool,
) -> AppMenuInput<'a> {
    AppMenuInput {
        signed_in: false,
        keymap: kerf_editor::prefs::Keymap::Kerf,
        commands,
        menus,
        states,
        checked,
        zoom: 0.0,
        collab,
        revision: 7,
        menu_bar,
        recent: &[],
        current: None,
    }
}

fn find_row<'a>(rows: &'a [AppRow], words: &str) -> &'a AppRow {
    rows.iter()
        .find(|row| row.words == words)
        .unwrap_or_else(|| panic!("no row {words:?}"))
}

#[test]
fn the_app_menus_mask_tools_are_ticked_while_out_and_greyed_with_why() {
    let commands = commands::catalog("kerf");
    let menus: Vec<String> = ["View".to_owned()].into();
    let mut states = json!({});
    for tool in MaskTool::ALL {
        states[mask_tool_id(tool)] = json!(NO_MASK_CLIP);
    }
    let checked = BTreeSet::new();
    // With the menu bar's menus repeated (Linux, Windows) and without.
    for menu_bar in [true, false] {
        let rows = app_rows(&kerf_menu_input(
            &commands, &menus, &states, &checked, None, menu_bar,
        ));
        let mut views = vec![find_row(&rows, "View")];
        if !menu_bar {
            views.extend(rows.iter().filter(|row| row.words == "View").skip(1));
        }
        for view in views {
            for tool in MaskTool::ALL {
                let row = find_row(&view.rows, tool.label());
                assert!(row.disabled, "{}", tool.label());
                assert_eq!(row.tip.as_deref(), Some(NO_MASK_CLIP));
                assert!(!row.checked);
                assert_eq!(row.does, AppDoes::Mask(tool));
            }
        }
    }
    // A picture clip chosen, the Ellipse out.
    let states = json!({});
    let checked: BTreeSet<String> = [mask_tool_id(MaskTool::Ellipse).to_owned()].into();
    let rows = app_rows(&kerf_menu_input(
        &commands, &menus, &states, &checked, None, false,
    ));
    for view in rows.iter().filter(|row| row.words == "View") {
        assert!(find_row(&view.rows, "Ellipse mask").checked);
        assert!(!find_row(&view.rows, "Rectangle mask").checked);
        assert!(!find_row(&view.rows, "Pen mask").disabled);
    }
}

#[test]
fn ask_kerf_is_greyed_with_why_where_the_app_menu_lists_it() {
    let commands = commands::catalog("kerf");
    let menus: Vec<String> = ["Clip".to_owned()].into();
    let context = json!({
        "connected": true, "revision": 3, "videoTracks": 1, "audioTracks": 1,
        "agentBusy": false, "aiAccess": "noKey",
    });
    let states = kerf_bridge::commands_states(&context);
    let checked = BTreeSet::new();
    let rows = app_rows(&kerf_menu_input(
        &commands, &menus, &states, &checked, None, false,
    ));
    let clip = find_row(&rows, "Clip");
    let ask = clip
        .rows
        .iter()
        .find(|row| row.does == AppDoes::Command("ai.ask".into()))
        .expect("Ask Kerf… in Clip");
    assert!(ask.disabled);
    assert_eq!(
        ask.tip.as_deref(),
        Some("Add your API key in Settings → Chat")
    );
    assert!(ask.key.is_some());
    // While Kerf works, the same.
    let busy = kerf_bridge::commands_states(&json!({
        "connected": true, "revision": 3, "agentBusy": true,
    }));
    let rows = app_rows(&kerf_menu_input(
        &commands, &menus, &busy, &checked, None, false,
    ));
    let ask = find_row(&rows, "Clip")
        .rows
        .iter()
        .find(|row| row.does == AppDoes::Command("ai.ask".into()))
        .unwrap();
    assert_eq!(ask.tip.as_deref(), Some("Kerf is working on a request"));
    // The menu bar greys it too.
    let menus = menus_with_states(
        "kerf",
        &[],
        None,
        &MenuStates {
            unavailable: ["ai.ask".to_owned()].into(),
            checked: BTreeSet::new(),
        },
    );
    let clip = menus.iter().find(|menu| menu.name == "Clip").unwrap();
    assert!(clip.items.iter().any(|item| matches!(
        item,
        MenuItem::Action { name, action, .. }
            if name == "Ask Kerf…" && action.partial_eq(&Unavailable)
    )));
}

#[test]
fn view_sync_says_why_the_last_sync_failed_until_one_succeeds() {
    let status = json!({
        "repository": true, "branch": "main", "branches": ["main"],
        "behind": 0, "ahead": 1, "conflicted": false,
    });
    // Nothing went wrong: no warning.
    assert_eq!(sync_problem(Some(&status)), None);
    assert_eq!(sync_problem(None), None);
    let failed = with_sync_failure(Some(&status), Some("could not reach origin")).unwrap();
    assert_eq!(failed[SYNC_FAILURE], "could not reach origin");
    assert_eq!(
        sync_problem(Some(&failed)).as_deref(),
        Some("Sync failed: could not reach origin")
    );
    // Succeeded since: the window gives none, and the warning goes.
    let synced = with_sync_failure(Some(&status), None).unwrap();
    assert_eq!(synced, status);
    assert_eq!(sync_problem(Some(&synced)), None);
    // No standing heard yet: nothing to say it under.
    assert_eq!(with_sync_failure(None, Some("no")), None);
    // An unfinished merge is said first: no sync goes until it is finished.
    let mut conflicted = failed.clone();
    conflicted["conflicted"] = json!(true);
    assert_eq!(
        sync_problem(Some(&conflicted)).as_deref(),
        Some("A merge is unfinished; finish it with git")
    );
    // In the Sync submenu, a greyed row under its summary.
    let commands = commands::catalog("kerf");
    let states = json!({});
    let checked = BTreeSet::new();
    let rows = app_rows(&kerf_menu_input(
        &commands,
        &[],
        &states,
        &checked,
        Some(&failed),
        true,
    ));
    let sync = find_row(&find_row(&rows, "View").rows, "Sync");
    assert_eq!(sync.rows[1].words, "Sync failed: could not reach origin");
    assert!(sync.rows[1].disabled);
    assert_eq!(sync.rows[1].does, AppDoes::Nothing);
    let rows = app_rows(&kerf_menu_input(
        &commands,
        &[],
        &states,
        &checked,
        Some(&synced),
        true,
    ));
    let sync = find_row(&find_row(&rows, "View").rows, "Sync");
    assert!(!sync
        .rows
        .iter()
        .any(|row| row.words.starts_with("Sync failed")));
}

// ------------------------------------------------------------ tips

proptest! {
    /// A control as tall as half the window or more (the sidebar's edge)
    /// has its tip by the pointer: the row it stands by is inside the
    /// control and holds the pointer where the pointer is on it.
    #[test]
    fn a_tall_control_has_its_tip_by_the_pointer(
        top in 0f32..200.0,
        tall in 1f32..1000.0,
        pointer in 0f32..1000.0,
    ) {
        let window = window(1600.0, 1000.0);
        let control = bounds(300.0, top, 8.0, tall);
        let at = point(px(304.0), px(pointer));
        let anchor = tip_anchor(control, at, window);
        if tall * 2.0 < 1000.0 {
            prop_assert_eq!(anchor, control);
        } else {
            prop_assert!(anchor.top() >= control.top() - px(0.01));
            prop_assert!(anchor.bottom() <= control.bottom() + px(0.01));
            let on = pointer >= top + layout::TIP_HEIGHT / 2.0
                && pointer <= top + tall - layout::TIP_HEIGHT / 2.0;
            if on {
                prop_assert!(anchor.top() <= px(pointer) && anchor.bottom() >= px(pointer));
            }
        }
    }
}

#[test]
fn a_tip_of_several_lines_goes_over_its_control_where_under_would_run_off() {
    let window = window(1600.0, 1000.0);
    // A tile's tip of five lines, the tile in the band low in a short window.
    let tip = size(px(300.0), px(tip_height(5)));
    assert_eq!(tip_height(1), layout::TIP_HEIGHT);
    assert_eq!(tip_height(3), layout::TIP_HEIGHT + 2.0 * text::CONTROL.line);
    let tile = bounds(400.0, 420.0, 136.0, 100.0);
    let short = size(px(1600.0), px(600.0));
    let at = tip_origin(tile, tip, short);
    assert!(f32::from(at.y) + tip_height(5) <= 420.0);
    // With room under it, it stays under.
    let at = tip_origin(tile, tip, window);
    assert!(f32::from(at.y) >= 520.0);
}

fn actions(menu: &gpui::Menu) -> Vec<(String, Box<dyn Action>)> {
    menu.items
        .iter()
        .filter_map(|item| match item {
            gpui::MenuItem::Action { name, action, .. } => {
                Some((name.to_string(), action.boxed_clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn switch_branch_in_the_menu_bar_is_a_submenu_of_the_other_branches() {
    let branches = MenuBranches {
        others: vec!["trailer".into(), "colour".into()],
        blocked: false,
    };
    let menus = menus_with_branches("kerf", &[], None, &MenuStates::default(), &branches);
    let collaborate = menus
        .iter()
        .find(|menu| menu.name == "Collaborate")
        .expect("a Collaborate menu");
    let switch = collaborate
        .items
        .iter()
        .find_map(|item| match item {
            gpui::MenuItem::Submenu(menu) if menu.name == "Switch Branch" => Some(menu),
            _ => None,
        })
        .expect("Switch Branch is a submenu");
    let rows = actions(switch);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "trailer");
    assert!(rows[0].1.partial_eq(&SwitchBranch {
        name: "trailer".into()
    }));
    assert!(rows[1].1.partial_eq(&SwitchBranch {
        name: "colour".into()
    }));
    // No row of Collaborate runs Switch Branch with no branch named.
    assert!(!actions(collaborate)
        .iter()
        .any(|(_, action)| action.partial_eq(&commands::RunCommand::new("collab.switchBranch"))));
    // With none, it says so, greyed (nothing carries it out).
    let alone = branch_menu(&MenuBranches::default());
    let rows = actions(&alone);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, NO_OTHER_BRANCHES);
    assert_eq!(rows[0].0, "No Other Branches");
    assert!(rows[0].1.partial_eq(&NoOtherBranches));
    // Held back (a sync cannot run now), each branch is greyed.
    let held = branch_menu(&MenuBranches {
        others: vec!["trailer".into()],
        blocked: true,
    });
    assert!(actions(&held)[0].1.partial_eq(&Unavailable));
}

// ---------------------------------------------------------------- startup

#[test]
fn window_sizes_read_as_width_x_height() {
    assert_eq!(parse_window_size("1600x1000"), Some((1600.0, 1000.0)));
    assert_eq!(parse_window_size(" 800X600 "), Some((800.0, 600.0)));
    for bad in ["", "1600", "x1000", "0x10", "ax b", "1600x1000x2", "-5x5"] {
        assert_eq!(parse_window_size(bad), None, "{bad}");
    }
}

#[test]
fn macos_path_gains_homebrews_and_local_tools_once() {
    let home = Path::new("/Users/jun");
    assert_eq!(
        path_with_tool_folders("/usr/bin:/bin", home),
        "/usr/bin:/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/Users/jun/.local/bin"
    );
    let full = path_with_tool_folders("/usr/local/bin::/opt/homebrew/bin", home);
    assert_eq!(
        full,
        "/usr/local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/Users/jun/.local/bin"
    );
    assert_eq!(path_with_tool_folders(&full, home), full, "nothing twice");
    // A home whose name has a colon cannot be written in a PATH: the others
    // still come, and once, so the editor starts again only once.
    let colon = Path::new("/Users/a:b");
    let with = path_with_tool_folders("/usr/bin", colon);
    assert_eq!(
        with,
        "/usr/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin"
    );
    assert_eq!(path_with_tool_folders(&with, colon), with);
}

#[test]
fn a_crash_is_told_of_once() {
    let listed = json!({"folder": "/c", "reports": [
        {"component": "kerfd", "atMs": 3000.0, "message": "index out of bounds\nat …"},
        {"component": "editor", "atMs": 2000.0, "message": ""},
        {"component": "kerf", "atMs": 1000.0, "message": ""},
    ]});
    let mut prefs = Prefs {
        crashes_seen_ms: 1000,
        ..Prefs::default()
    };
    let unseen = prefs.unseen_crashes(&listed);
    assert_eq!(unseen.len(), 2);
    let text = crash_notice_text(&unseen);
    assert!(text.starts_with(
        "Kerf stopped unexpectedly 2 times since it last said so. The latest was in Kerf's project service (kerfd)."
    ));
    assert!(text.contains("“index out of bounds”"), "{text}");
    assert!(text.contains("crash folder, on this computer only"));
    let one = crash_notice_text(&unseen[..1]);
    assert!(one.starts_with(
        "Kerf stopped unexpectedly: Kerf's project service (kerfd) hit an error it could not recover from."
    ));
    prefs.saw_crashes(&unseen);
    assert_eq!(prefs.crashes_seen_ms, 3000);
    assert!(prefs.unseen_crashes(&listed).is_empty());
    assert_eq!(component_name("kerf"), "the kerf command");
    assert_eq!(component_name("editor"), "the editor");
}

#[test]
fn every_menu_is_one_of_the_registrys() {
    let menus = kerf_editor::keymap::menus("kerf");
    assert_eq!(menus[0].name.as_ref(), "Industrial Kerf");
    let names: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
    for expected in ["File", "Edit", "Playback", "View", "Help"] {
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
}

// ---------------------------------------------------------------- --screenshot's pixels

/// The usual X server layout: 32 bits a pixel, BGRX in memory.
const XRGB: X11Pixels = X11Pixels {
    bits_per_pixel: 32,
    scanline_pad: 32,
    red_mask: 0x00ff_0000,
    green_mask: 0x0000_ff00,
    blue_mask: 0x0000_00ff,
    msb_first: false,
};

#[test]
fn x11_pixels_of_other_layouts_are_read() {
    // 24 bits a pixel, rows padded to 32 bits: 2 pixels take 8 bytes a row.
    let packed = X11Pixels {
        bits_per_pixel: 24,
        ..XRGB
    };
    let data = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0xaa, 0xbb, // row 0
        0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0xcc, 0xdd, // row 1
    ];
    let picture = x11_rgba(&data, 2, 2, packed).unwrap();
    assert_eq!(picture.get_pixel(0, 0).0, [0x03, 0x02, 0x01, 255]);
    assert_eq!(picture.get_pixel(1, 1).0, [0x60, 0x50, 0x40, 255]);
    // 16 bits, 5-6-5, most significant byte first.
    let wide = X11Pixels {
        bits_per_pixel: 16,
        scanline_pad: 16,
        red_mask: 0xf800,
        green_mask: 0x07e0,
        blue_mask: 0x001f,
        msb_first: true,
    };
    let picture = x11_rgba(&[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f], 3, 1, wide).unwrap();
    assert_eq!(picture.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(picture.get_pixel(1, 0).0, [0, 255, 0, 255]);
    assert_eq!(picture.get_pixel(2, 0).0, [0, 0, 255, 255]);
    // Too few bytes, or a depth it does not know: nothing.
    assert!(x11_rgba(&data[..10], 2, 2, packed).is_none());
    let odd = X11Pixels {
        bits_per_pixel: 8,
        ..XRGB
    };
    assert!(x11_rgba(&[0; 16], 4, 4, odd).is_none());
}

proptest! {
    #[test]
    fn x11_pixels_read_back_as_they_were(
        width in 1u32..9,
        rows in prop::collection::vec(prop::collection::vec(any::<[u8; 3]>(), 8), 1..6),
        msb_first in any::<bool>(),
    ) {
        let height = rows.len() as u32;
        let format = X11Pixels { msb_first, ..XRGB };
        let mut data = Vec::new();
        for row in &rows {
            for [red, green, blue] in row.iter().take(width as usize) {
                let pixel = u32::from(*red) << 16 | u32::from(*green) << 8 | u32::from(*blue);
                let bytes = if msb_first { pixel.to_be_bytes() } else { pixel.to_le_bytes() };
                data.extend_from_slice(&bytes);
            }
        }
        let picture = x11_rgba(&data, width, height, format).unwrap();
        for (y, row) in rows.iter().enumerate() {
            for (x, [red, green, blue]) in row.iter().take(width as usize).enumerate() {
                prop_assert_eq!(
                    picture.get_pixel(x as u32, y as u32).0,
                    [*red, *green, *blue, 255]
                );
            }
        }
    }
}

#[test]
fn notices_stand_above_the_composer_and_what_stands_over_it() {
    use kerf_editor::workspace::notice_bottom;
    // The composer with Kerf's next steps over it starts 160 up a window
    // 1000 tall: a notice's foot stands NOTICE_GAP above that.
    assert_eq!(
        notice_bottom(1000.0, Some(840.0)),
        160.0 + theme::layout::NOTICE_GAP
    );
    // Before the composer has been laid out, where notices always stood.
    assert_eq!(notice_bottom(1000.0, None), theme::layout::NOTICE_FOOT);
}

proptest::proptest! {
    /// Wherever the composer's stack starts, a notice's foot is above it,
    /// by the gap, and inside the window; a taller stack lifts it as much.
    #[test]
    fn a_notice_never_covers_the_composer_s_stack(
        height in 200.0f32..2400.0,
        share in 0.0f32..1.0,
        grow in 0.0f32..200.0,
    ) {
        use kerf_editor::workspace::notice_bottom;
        let top = height * share;
        let bottom = notice_bottom(height, Some(top));
        let foot = height - bottom;
        proptest::prop_assert!((top - foot - theme::layout::NOTICE_GAP).abs() < 1e-3);
        proptest::prop_assert!(bottom >= theme::layout::NOTICE_GAP);
        let higher = (top - grow).max(0.0);
        let lifted = notice_bottom(height, Some(higher));
        proptest::prop_assert!((lifted - bottom - (top - higher)).abs() < 1e-3);
    }
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

fn premiere_menu_input<'a>(
    commands: &'a [commands::Command],
    menus: &'a [String],
    states: &'a Value,
    checked: &'a BTreeSet<String>,
    collab: Option<&'a Value>,
    menu_bar: bool,
) -> AppMenuInput<'a> {
    AppMenuInput {
        signed_in: false,
        keymap: kerf_editor::prefs::Keymap::Premiere,
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
    let rows = app_rows(&premiere_menu_input(
        &commands,
        &[],
        &states,
        &checked,
        None,
        true,
    ));
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
    assert_eq!(
        ticked,
        [&AppDoes::Keymap(kerf_editor::prefs::Keymap::Premiere)]
    );
    // With a menu bar the registry's menus are not repeated.
    assert!(!rows.iter().any(|row| row.words == "File"));
}

#[test]
fn without_a_menu_bar_the_registrys_menus_are_in_the_app_menu() {
    let commands = commands::catalog("kerf");
    let menus: Vec<String> = ["File", "Edit", "Help"].map(str::to_owned).to_vec();
    let states = json!({"edit.undo": "Nothing to undo"});
    let checked = BTreeSet::new();
    let rows = app_rows(&premiere_menu_input(
        &commands, &menus, &states, &checked, None, false,
    ));
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
    let rows = app_rows(&premiere_menu_input(
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
    let none = app_rows(&premiere_menu_input(
        &commands,
        &[],
        &states,
        &checked,
        None,
        true,
    ));
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
    let mut input = premiere_menu_input(&commands, &menus, &states, &checked, None, false);
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
        quality: Some(RenderQuality::from_name(quality)),
        status: RenderStatus::from_name(status),
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
    let rows = app_rows(&premiere_menu_input(
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
    let rows = app_rows(&premiere_menu_input(
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

// ---------------------------------------------------------------- the window

/// A screen at `(left, top)`, `width` × `height` points.
fn screen(left: f32, top: f32, width: f32, height: f32) -> Bounds<gpui::Pixels> {
    Bounds::new(point(px(left), px(top)), size(px(width), px(height)))
}

/// `bounds` as (left, top, width, height) in points.
fn sides(bounds: Bounds<gpui::Pixels>) -> (f32, f32, f32, f32) {
    (
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

/// Whether `inner` lies on `outer`, edges and all.
fn inside(inner: Bounds<gpui::Pixels>, outer: Bounds<gpui::Pixels>) -> bool {
    let ((x, y, w, h), (ox, oy, ow, oh)) = (sides(inner), sides(outer));
    x >= ox - 1e-3 && y >= oy - 1e-3 && x + w <= ox + ow + 1e-3 && y + h <= oy + oh + 1e-3
}

#[test]
fn a_window_taking_anothers_place_opens_where_it_was() {
    let there = Bounds::new(point(px(200.0), px(120.0)), size(px(1100.0), px(760.0)));
    // At its size and place, maximized or full screen as it was, whatever
    // the screen and the size asked.
    for place in [
        WindowBounds::Windowed(there),
        WindowBounds::Maximized(there),
        WindowBounds::Fullscreen(there),
    ] {
        for asked in [None, Some(size(px(1600.0), px(1000.0)))] {
            for shown in [None, Some(screen(0.0, 0.0, 1280.0, 720.0))] {
                assert_eq!(window_place(Some(place), asked, shown), place);
            }
        }
    }
    assert_eq!(layout::WINDOW, (1500.0, 940.0));
}

#[test]
fn a_first_window_fills_a_screen_too_small_for_it() {
    // 1366 × 768, and 1920 × 1080 at 150% (1280 × 720): maximized, and put
    // back it fits on the screen with its room kept, in the middle; never
    // off its top or left edge, where the toolbar would be cut.
    for (width, height) in [(1366.0, 768.0), (1280.0, 720.0), (1440.0, 900.0)] {
        let shown = screen(0.0, 0.0, width, height);
        let WindowBounds::Maximized(back) = window_place(None, None, Some(shown)) else {
            panic!("{width} × {height}: fills the screen");
        };
        let (x, y, w, h) = sides(back);
        assert_eq!(w, width - layout::SCREEN_KEPT.0);
        assert_eq!(h, (height - layout::SCREEN_KEPT.1).min(940.0));
        assert!(inside(back, shown));
        assert!((x - (width - w) / 2.0).abs() < 1e-3 && (y - (height - h) / 2.0).abs() < 1e-3);
    }
    // Too short only: still filled, put back as wide as a first window where
    // that fits.
    let WindowBounds::Maximized(back) =
        window_place(None, None, Some(screen(0.0, 0.0, 1920.0, 1000.0)))
    else {
        panic!("fills a short screen");
    };
    assert_eq!(sides(back).2, 1500.0);
}

#[test]
fn a_size_asked_is_kept_and_starts_on_the_screen() {
    let asked = size(px(1600.0), px(1000.0));
    // Where it fits, in the middle.
    assert_eq!(
        window_place(None, Some(asked), Some(screen(0.0, 0.0, 1920.0, 1200.0))),
        WindowBounds::Windowed(screen(160.0, 100.0, 1600.0, 1000.0))
    );
    // As big as the screen: at its corner, windowed, as asked.
    assert_eq!(
        window_place(None, Some(asked), Some(screen(0.0, 0.0, 1600.0, 1000.0))),
        WindowBounds::Windowed(screen(0.0, 0.0, 1600.0, 1000.0))
    );
    // Bigger: still that size, from the screen's top left corner.
    assert_eq!(
        window_place(None, Some(asked), Some(screen(0.0, 0.0, 1280.0, 720.0))),
        WindowBounds::Windowed(screen(0.0, 0.0, 1600.0, 1000.0))
    );
}

proptest! {
    /// Following no window, the window (or where it is put back to) is
    /// on the screen; windowed only at `layout::WINDOW` with the room kept, else
    /// filling the screen.
    #[test]
    fn a_first_window_is_on_the_screen(
        left in -3000.0f32..3000.0,
        top in -2000.0f32..2000.0,
        width in 800.0f32..6000.0,
        height in 600.0f32..4000.0,
    ) {
        let shown = screen(left, top, width, height);
        let (kept_width, kept_height) = layout::SCREEN_KEPT;
        let fits = 1500.0 + kept_width <= width && 940.0 + kept_height <= height;
        match window_place(None, None, Some(shown)) {
            WindowBounds::Windowed(bounds) => {
                prop_assert!(fits);
                let (x, y, w, h) = sides(bounds);
                prop_assert_eq!((w, h), (1500.0, 940.0));
                prop_assert!(inside(bounds, shown));
                // In the middle.
                prop_assert!((x - left - (width - w) / 2.0).abs() < 1e-2);
                prop_assert!((y - top - (height - h) / 2.0).abs() < 1e-2);
            }
            WindowBounds::Maximized(back) => {
                prop_assert!(!fits);
                let (_, _, w, h) = sides(back);
                prop_assert!(w <= 1500.0 && h <= 940.0);
                prop_assert!(w >= layout::WINDOW_LEAST.0 && h >= layout::WINDOW_LEAST.1);
                prop_assert!(inside(back, shown));
            }
            other => prop_assert!(false, "{other:?}"),
        }
    }

    /// A size asked is kept, windowed, never starting off the screen's top
    /// or left edge, and in the middle where it fits.
    #[test]
    fn a_size_asked_never_starts_off_the_screen(
        left in -3000.0f32..3000.0,
        top in -2000.0f32..2000.0,
        width in 800.0f32..6000.0,
        height in 600.0f32..4000.0,
        asked_width in 1u32..8000,
        asked_height in 1u32..8000,
    ) {
        let asked = size(px(asked_width as f32), px(asked_height as f32));
        let shown = screen(left, top, width, height);
        let WindowBounds::Windowed(bounds) = window_place(None, Some(asked), Some(shown)) else {
            panic!("a size asked is windowed");
        };
        let (x, y, w, h) = sides(bounds);
        prop_assert_eq!((w, h), (asked_width as f32, asked_height as f32));
        prop_assert!(x >= left - 1e-3 && y >= top - 1e-3);
        if w <= width && h <= height {
            prop_assert!(inside(bounds, shown));
        }
    }
}

#[test]
fn the_window_buttons_stand_on_the_toolbars_line() {
    let at = window_buttons_at();
    assert_eq!(f32::from(at.x), layout::WINDOW_BUTTONS_LEFT);
    // On the middle of the toolbar's height.
    let middle = f32::from(at.y) + layout::WINDOW_BUTTON / 2.0;
    assert!((middle - layout::TOOLBAR / 2.0).abs() < 1e-3);
    // The first control starts past the three buttons (23 apart), a step on.
    const { assert!(layout::WINDOW_BUTTONS_ROOM > layout::WINDOW_BUTTONS_LEFT + 2.0 * 23.0 + 14.0) };
    let bar = titlebar("Atlas");
    assert_eq!(
        bar.title.as_ref().map(|title| title.as_ref()),
        Some("Atlas")
    );
    // The toolbar is the title bar on macOS only; elsewhere the system's stays.
    assert_eq!(bar.appears_transparent, cfg!(target_os = "macos"));
    assert_eq!(
        bar.traffic_light_position.is_some(),
        cfg!(target_os = "macos")
    );
}

#[test]
fn the_toolbars_menus_open_under_the_controls_past_the_window_buttons() {
    // With no window buttons beside it (off macOS, full screen): where
    // they always were, ⋯ at 40 and the title at 84.
    let plain = toolbar_places(true, 0.0);
    assert_eq!(
        plain,
        ToolbarPlaces {
            lead: 8.0,
            title: 84.0,
            app_menu: 40.0
        }
    );
    // Past the window buttons, the controls and the menus they open move
    // on together.
    let past = toolbar_places(true, layout::WINDOW_BUTTONS_ROOM);
    let on = layout::WINDOW_BUTTONS_ROOM - 8.0;
    assert_eq!(past.lead, layout::WINDOW_BUTTONS_ROOM);
    assert_eq!(past.title, plain.title + on);
    assert_eq!(past.app_menu, plain.app_menu + on);
    // With the sidebar out the toolbar is not at the window's edge: no
    // room, and the title first.
    let beside = toolbar_places(false, layout::WINDOW_BUTTONS_ROOM);
    assert_eq!((beside.lead, beside.title), (8.0, 16.0));
    // The sidebar's top leaves the same room as the toolbar's.
    assert_eq!(lead_past(layout::WINDOW_BUTTONS_ROOM), past.lead);
    assert_eq!(lead_past(0.0), 8.0);
}

proptest! {
    /// Whatever room the window buttons need, the toolbar's menus keep
    /// their place under the controls that open them, and its first
    /// control is past the buttons.
    #[test]
    fn the_toolbars_menus_follow_its_controls(room in 0.0f32..400.0, leading: bool) {
        let places = toolbar_places(leading, room);
        let plain = toolbar_places(leading, 0.0);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        prop_assert!(near(places.title - places.lead, plain.title - plain.lead));
        prop_assert!(near(places.app_menu - places.lead, plain.app_menu - plain.lead));
        if leading {
            prop_assert!(places.lead >= room && places.lead >= 8.0);
        } else {
            prop_assert_eq!(places, plain);
        }
    }
}

// ----------------------------------------------------- edited, how long ago

/// Seconds since 1970 of `text` (RFC 3339).
fn at(text: &str) -> i64 {
    text.parse::<jiff::Timestamp>().unwrap().as_second()
}

proptest! {
    /// "Edited … ago" says the same until the moment it is said to change,
    /// summer time or not, and in UTC something else from then; once it
    /// names the date, it stays.
    #[test]
    fn edited_ago_changes_when_it_is_said_to(
        then in 1_600_000_000i64..1_900_000_000,
        back in prop_oneof![-100i64..3600, 3600i64..86_400, 86_400i64..700_000, 700_000i64..2_000_000],
    ) {
        let now = then + back;
        let california = TimeZone::posix("PST8PDT,M3.2.0,M11.1.0").unwrap();
        for (zone, exact) in [(TimeZone::UTC, true), (california, false)] {
            let said = edited_ago_in(then, now, &zone);
            match edited_ago_changes_in(then, now, &zone) {
                Some(next) => {
                    prop_assert!(next > now);
                    prop_assert_eq!(edited_ago_in(then, next - 1, &zone), said.clone());
                    if exact {
                        prop_assert_ne!(edited_ago_in(then, next, &zone), said);
                    }
                }
                None => prop_assert_eq!(edited_ago_in(then, now + 400 * 86_400, &zone), said),
            }
        }
    }
}

#[test]
fn edited_ago_counts_days_on_the_local_calendar() {
    let pacific = TimeZone::fixed(jiff::tz::offset(-8));
    // 23:30 and 00:30 in California: yesterday there, an hour ago in UTC.
    let then = at("2026-11-21T07:30:00Z");
    let now = at("2026-11-21T08:30:00Z");
    assert_eq!(edited_ago_in(then, now, &pacific), "yesterday");
    assert_eq!(edited_ago_in(then, now, &TimeZone::UTC), "1 h ago");
    // And the other way round east of UTC: the same day in Tokyo.
    let tokyo = TimeZone::fixed(jiff::tz::offset(9));
    let then = at("2026-11-20T16:00:00Z");
    let now = at("2026-11-21T02:00:00Z");
    assert_eq!(edited_ago_in(then, now, &tokyo), "10 h ago");
    assert_eq!(edited_ago_in(then, now, &TimeZone::UTC), "yesterday");
    // A week on, the date as it was where the person is, summer time and all.
    let california = TimeZone::posix("PST8PDT,M3.2.0,M11.1.0").unwrap();
    let then = at("2026-03-01T02:00:00Z");
    let now = at("2026-03-20T12:00:00Z");
    assert_eq!(edited_ago_in(then, now, &california), "28 Feb 2026");
    assert_eq!(edited_ago_in(then, now, &TimeZone::UTC), "1 Mar 2026");
    let then = at("2026-07-01T05:30:00Z");
    assert_eq!(
        edited_ago_in(then, at("2026-07-30T00:00:00Z"), &california),
        "30 Jun 2026"
    );
}

proptest! {
    #[test]
    fn the_day_count_is_the_calendars(
        then in 0i64..4_000_000_000,
        later in 0i64..40 * 86_400,
        hours in -12i8..=14,
        summer in any::<bool>(),
    ) {
        let zone = if summer {
            TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap()
        } else {
            TimeZone::fixed(jiff::tz::offset(hours))
        };
        let now = then + later;
        let date = |seconds: i64| {
            jiff::Timestamp::from_second(seconds).unwrap().to_zoned(zone.clone()).date()
        };
        let epoch = jiff::civil::date(1970, 1, 1);
        // The day each falls on is the calendar's own...
        for seconds in [then, now] {
            let days = epoch.until(date(seconds)).unwrap().get_days();
            prop_assert_eq!(local_day(seconds, &zone), i64::from(days));
        }
        // ...and never goes back as time goes on.
        prop_assert!(local_day(now, &zone) >= local_day(then, &zone));
        let days = local_day(now, &zone) - local_day(then, &zone);
        let said = edited_ago_in(then, now, &zone);
        let calendar_days = i64::from(date(then).until(date(now)).unwrap().get_days());
        prop_assert_eq!(days, calendar_days);
        if later >= 3600 {
            prop_assert_eq!(said == "yesterday", days == 1 || (days == 0 && later >= 86_400));
            prop_assert_eq!(said.ends_with(" h ago"), days == 0 && later < 86_400);
            if (2..7).contains(&days) {
                prop_assert_eq!(said, format!("{days} days ago"));
            }
        }
    }
}

#[test]
fn the_error_bar_reads_as_a_person_does() {
    assert_eq!(
        readable_error("error[ripple.blocked]: The clip after it is locked"),
        "The clip after it is locked"
    );
    assert_eq!(
        readable_error("warning[edit.range-clamp] 3:14: Clamped to the clip"),
        "Clamped to the clip"
    );
    assert_eq!(
        readable_error("Runtime is disconnected"),
        "Runtime is disconnected"
    );
    // Not a diagnostic's prefix: left as it is.
    assert_eq!(readable_error("error[]: x"), "error[]: x");
    assert_eq!(readable_error("error[a b]: x"), "error[a b]: x");
    assert_eq!(readable_error("error[code]: "), "error[code]: ");
}

#[test]
fn what_a_view_answers_in_place_stays_out_of_the_error_bar() {
    for command in [
        "media_make",
        "takes_read",
        "recipe_save",
        "shot_read",
        "workflow_run",
    ] {
        assert!(!kept_in_bar(command), "{command}");
    }
    for command in ["", "agent", "edit", "forward", "conversation"] {
        assert!(kept_in_bar(command), "{command}");
    }
}

#[test]
fn the_window_is_titled_by_its_project_and_a_sequence_named_otherwise() {
    use kerf_editor::model::{Timeline, TimelineShow};
    use kerf_editor::workspace::window_title;
    let timeline = |name: &str| {
        Timeline::from_value(&serde_json::json!({
            "name": "Atlas", "sequence": "s1",
            "sequences": [{"id": "s1", "name": name}, {"id": "s2", "name": "Other"}],
        }))
    };
    assert_eq!(
        window_title("Atlas", &timeline("30 s spot v7")),
        "Atlas · 30 s spot v7"
    );
    assert_eq!(window_title("Atlas", &timeline("Atlas")), "Atlas");
    assert_eq!(window_title("Atlas", &timeline("")), "Atlas");
    assert_eq!(window_title("Atlas", &Timeline::default()), "Atlas");
}

#[test]
fn open_recent_checks_the_project_here_and_greys_a_folder_that_is_gone() {
    let here = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let gone = PathBuf::from("/nowhere/Kerf/Gone Project");
    let recent = vec![
        here.path().to_path_buf(),
        gone.clone(),
        other.path().to_path_buf(),
    ];
    let rows = recent_rows(&recent, Some(here.path()));
    assert_eq!(rows.len(), 3);
    assert!(rows[0].current && !rows[0].missing);
    assert_eq!(
        rows[1],
        RecentRow {
            name: "Gone Project".into(),
            path: gone,
            current: false,
            missing: true,
        }
    );
    assert!(!rows[2].current && !rows[2].missing);

    let names = |menu: &gpui::Menu| -> Vec<String> {
        menu.items
            .iter()
            .map(|item| match item {
                MenuItem::Action { name, .. } => name.to_string(),
                MenuItem::Separator => "—".to_owned(),
                MenuItem::Submenu(menu) => format!("{} ▸", menu.name),
                MenuItem::SystemMenu(_) => "system".to_owned(),
            })
            .collect()
    };
    let menu = recent_menu(&recent, Some(here.path()));
    let listed = names(&menu);
    assert_eq!(menu.name.as_ref(), "Open Recent");
    assert!(listed[0].starts_with("✓ "), "{listed:?}");
    assert_eq!(listed[1], "Gone Project");
    assert_eq!(&listed[3..], ["—", "Clear Menu"]);

    let empty = recent_menu(&[], None);
    assert_eq!(names(&empty), ["No Recent Projects", "—", "Clear Menu"]);
}

#[test]
fn open_recent_follows_open_project_in_the_file_menu() {
    let menus = menus_with("kerf", &[], None);
    let file = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "File")
        .expect("a File menu");
    let at = file
        .items
        .iter()
        .position(|item| matches!(item, MenuItem::Action { name, .. } if name.as_ref() == "Open Project…"))
        .expect("Open Project…");
    assert!(
        matches!(&file.items[at + 1], MenuItem::Submenu(menu) if menu.name.as_ref() == "Open Recent")
    );
}
