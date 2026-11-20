//! The window's toolbar and File menu, through their public functions:
//! where an export goes, what Activity keeps, what File ▸ Open Recent
//! lists, how a shortcut is written, and where a finished export's frame
//! is on its way home.

use gpui::{point, px, size, Bounds, MenuItem};
use kerf_editor::commands::{
    catalog, keystroke, menus_with, recent_menu, recent_rows, shown_keys, RecentRow,
};
use kerf_editor::export::{export_tip, Quality};
use kerf_editor::model::Proposal;
use kerf_editor::palette::{shortcut, unavailable};
use kerf_editor::session::{
    activity_of, export_request, proposal_context, ActivityLog, ACTIVITY_KEPT, DELIVERIES,
};
use kerf_editor::theme::motion;
use kerf_editor::workspace::{export_flight_at, lerp_bounds, picture_within};
use proptest::prelude::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[test]
fn a_master_and_a_draft_go_into_deliveries_named_after_the_folder() {
    let project = Path::new("/films/Atlas");
    assert_eq!(
        export_request(Some(project), "master"),
        json!({"type": "export", "quality": "master", "output": DELIVERIES, "name": "Atlas_master"})
    );
    assert_eq!(
        export_request(Some(project), Quality::Draft.wire()),
        json!({"type": "export", "quality": "review", "codec": "h264", "output": DELIVERIES,
            "name": "Atlas_draft"})
    );
    // Other qualities, and a project with no folder name, go where kerfd puts them.
    assert_eq!(
        export_request(Some(project), "preview"),
        json!({"type": "export", "quality": "preview"})
    );
    assert_eq!(
        export_request(None, "review"),
        json!({"type": "export", "quality": "review", "codec": "h264"})
    );
}

#[test]
fn the_export_button_names_its_quality_and_why_it_cannot_render() {
    assert_eq!(Quality::default(), Quality::Master);
    assert_eq!(Quality::Master.wire(), "master");
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
        for (quality, suffix) in [("master", "master"), ("review", "draft")] {
            let request = export_request(Some(&project), quality);
            prop_assert_eq!(request["output"].as_str(), Some(DELIVERIES));
            let name = format!("{folder}_{suffix}");
            prop_assert_eq!(request["name"].as_str(), Some(name.as_str()));
        }
    }

    #[test]
    fn activity_keeps_the_newest_lines_in_order(count in 0usize..200) {
        let mut log = ActivityLog::default();
        for index in 0..count {
            log.push("export", &format!("line {index}"));
        }
        prop_assert_eq!(log.len(), count.min(ACTIVITY_KEPT));
        let shown: Vec<String> = log.latest(8).map(|(_, text)| text.to_owned()).collect();
        let expected: Vec<String> = (count.saturating_sub(8)..count)
            .map(|index| format!("line {index}"))
            .collect();
        prop_assert_eq!(shown, expected);
    }
}

#[test]
fn activity_says_what_an_export_made_and_keeps_nothing_empty() {
    let mut log = ActivityLog::default();
    log.push("import", "   ");
    assert!(log.is_empty());
    assert_eq!(
        activity_of(
            &json!({"type": "export", "status": "completed", "jobId": "j1",
            "path": "/films/Atlas/deliveries/Atlas_master.mov"})
        ),
        Some((
            "export",
            "/films/Atlas/deliveries/Atlas_master.mov".to_owned()
        ))
    );
    assert_eq!(
        activity_of(&json!({"type": "export", "status": "failed", "message": "disk full"})),
        Some(("export", "Export failed: disk full".to_owned()))
    );
    assert_eq!(
        activity_of(&json!({"type": "export", "status": "running", "progress": 0.4})),
        None
    );
    assert_eq!(
        activity_of(&json!({"type": "analysis", "message": "Analysed motion for clip 2"})),
        Some(("analysis", "Analysed motion for clip 2".to_owned()))
    );
    assert_eq!(
        activity_of(&json!({"type": "clock", "position": 2.0})),
        None
    );
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

#[test]
fn a_shortcut_is_written_as_the_system_writes_it() {
    if cfg!(target_os = "macos") {
        assert_eq!(shown_keys("Ctrl+E"), "⌘E");
        assert_eq!(shown_keys("Ctrl+Shift+Z"), "⇧⌘Z");
        assert_eq!(shown_keys("Alt+Ctrl+S"), "⌥⌘S");
    } else {
        assert_eq!(shown_keys("Ctrl+E"), "Ctrl+E");
        assert_eq!(shown_keys("Ctrl+Shift+Z"), "Ctrl+Shift+Z");
        // Meta+ is Ctrl+ off macOS, where the system keeps Super for itself.
        assert_eq!(keystroke("Meta+R").as_deref(), Some("ctrl-r"));
    }
}

#[test]
fn a_finished_exports_frame_crosses_fading_then_fades_where_it_lands() {
    let (along, opacity) = export_flight_at(Duration::ZERO).unwrap();
    assert_eq!(along, 0.0);
    assert!((opacity - 0.9).abs() < 1e-4);
    let (along, opacity) = export_flight_at(motion::FLIGHT).unwrap();
    assert!((along - 1.0).abs() < 1e-4);
    assert!((opacity - 0.2).abs() < 1e-4);
    let (_, opacity) = export_flight_at(motion::FLIGHT + motion::FLIGHT_LAND / 2).unwrap();
    assert!((opacity - 0.1).abs() < 1e-3);
    assert!(export_flight_at(motion::FLIGHT + motion::FLIGHT_LAND).is_none());
}

proptest! {
    #[test]
    fn the_frame_never_brightens_and_ends_where_it_lands(ms in 0u64..1200) {
        let now = Duration::from_millis(ms);
        let later = now + Duration::from_millis(16);
        if let (Some((_, now)), Some((_, later))) = (export_flight_at(now), export_flight_at(later)) {
            prop_assert!(later <= now + 1e-6);
        }
        let from = Bounds::new(point(px(600.0), px(200.0)), size(px(640.0), px(360.0)));
        let to = Bounds::new(point(px(12.0), px(118.0)), size(px(44.0), px(26.0)));
        prop_assert_eq!(lerp_bounds(from, to, 0.0), from);
        prop_assert_eq!(lerp_bounds(from, to, 1.0), to);
    }

    #[test]
    fn the_picture_flies_from_inside_the_stage_at_its_own_shape(
        width in 200f32..2000.0,
        height in 200f32..1400.0,
        frame_w in 16u32..4000,
        frame_h in 16u32..4000,
    ) {
        let slot = Bounds::new(point(px(400.0), px(80.0)), size(px(width), px(height)));
        let picture = picture_within(slot, frame_w, frame_h);
        let (w, h) = (f32::from(picture.size.width), f32::from(picture.size.height));
        prop_assert!(f32::from(picture.origin.x) >= 400.0 - 0.01);
        prop_assert!(f32::from(picture.origin.y) >= 80.0 - 0.01);
        prop_assert!(f32::from(picture.origin.x) + w <= 400.0 + width + 0.01);
        prop_assert!(f32::from(picture.origin.y) + h <= 80.0 + height + 0.01);
        let aspect = frame_w as f32 / frame_h as f32;
        prop_assert!(((w / h) - aspect).abs() / aspect < 1e-3);
    }
}

/// What File ▸ Export… asks the registry for: the chosen quality as Qt's
/// File ▸ Export… named it, rendered where kerfd puts it, not the
/// toolbar's delivery into deliveries/.
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
    let toolbar = export_request(Some(Path::new("/films/Atlas")), Quality::Master.wire());
    assert_eq!(toolbar["quality"], "master");
    assert_eq!(toolbar["output"], DELIVERIES);
}

fn proposal(freshness: &str) -> Proposal {
    serde_json::from_value(json!({
        "proposalId": "p1",
        "status": "pending",
        "freshness": freshness,
    }))
    .unwrap()
}

#[test]
fn a_stale_proposal_can_be_brought_forward_and_accept_says_how() {
    let states = |proposal: &Proposal| {
        kerf_bridge::commands_states(&json!({
            "connected": true,
            "proposal": proposal_context(proposal),
        }))
    };
    let stale = states(&proposal("stale"));
    assert_eq!(unavailable(&stale, "proposal.forward"), None);
    let accept = unavailable(&stale, "proposal.accept").unwrap();
    assert!(accept.contains("bring it forward"), "{accept}");
    assert_eq!(unavailable(&stale, "proposal.reject"), None);

    let fresh = states(&proposal("fresh"));
    assert_eq!(unavailable(&fresh, "proposal.accept"), None);
    assert_eq!(
        unavailable(&fresh, "proposal.forward"),
        Some("This proposal is already on the edit as it is")
    );
    // Bring Proposal Forward resolves to its request for the stale one.
    let forward = kerf_bridge::commands_run(&json!({
        "command": "proposal.forward",
        "context": {"connected": true, "proposal": proposal_context(&proposal("stale"))},
        "params": {},
    }));
    assert!(forward["request"].is_object(), "{forward}");
}

#[test]
fn the_palette_writes_a_shortcut_as_the_window_binds_it() {
    let render = catalog("fcp")
        .into_iter()
        .find(|command| command.id == "sequence.renderPreview")
        .unwrap();
    assert_eq!(render.keys.first().map(String::as_str), Some("Meta+R"));
    if cfg!(target_os = "macos") {
        assert_eq!(shortcut(&render), "⌃R");
    } else {
        assert_eq!(shortcut(&render), "Ctrl+R");
        assert_eq!(shown_keys("Meta+Shift+R, Meta+K"), "Ctrl+Shift+R, Ctrl+K");
        assert_eq!(shown_keys("Ctrl++"), "Ctrl++");
    }
}

#[test]
fn every_shortcut_as_written_binds_the_keys_it_is_bound_to() {
    // Off macOS what the palette and tips write reads back as the same
    // keys the window binds, with no Meta left in it.
    if cfg!(target_os = "macos") {
        return;
    }
    // The keys a shortcut presses, as GPUI reads them.
    let pressed = |shortcut: &str| -> Option<Vec<gpui::Keystroke>> {
        keystroke(shortcut)?
            .split_whitespace()
            .map(|part| gpui::Keystroke::parse(part).ok())
            .collect()
    };
    for keymap in ["kerf", "premiere", "fcp"] {
        for command in catalog(keymap) {
            for key in &command.keys {
                let shown = shown_keys(key);
                assert!(!shown.to_lowercase().contains("meta"), "{keymap}: {shown}");
                let bound = pressed(key);
                assert!(bound.is_some(), "{keymap}: {key}");
                assert_eq!(pressed(&shown), bound, "{keymap}: {key} as {shown}");
            }
        }
    }
}
