//! What the window does with what the views hand it: an error's bar, a
//! notice's stay, the panel's split by the picture's shape, a frame step,
//! and the ways to another project (File ▸ Open Recent in the palette, the
//! mask tools in View). Through the public interface only.

use gpui::MenuItem;
use gpui::{point, px, size, Bounds};
use kerf_editor::commands::{catalog, menus_with};
use kerf_editor::palette::{matches, recent_commands, recent_path, RECENT_MENU};
use kerf_editor::store::EditorState;
use kerf_editor::theme::{layout, motion};
use kerf_editor::workspace::{first_line, frame_step, landing_in, timeline_split};
use proptest::prelude::*;
use std::time::Duration;

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
    // Under a wide picture Qt's 56%, the transport counted in.
    let qt_wide = height * layout::TIMELINE_WIDE_SHARE;
    assert!((wide + layout::TRANSPORT - qt_wide).abs() < 0.5, "{wide}");
    let qt_vertical = height * layout::TIMELINE_UPRIGHT_SHARE;
    assert!(
        (vertical + layout::TRANSPORT - qt_vertical).abs() < 0.5,
        "{vertical}"
    );
    assert!(vertical < wide);
    // A wide picture in a narrow stage leaves the timeline what it cannot use.
    let narrow = timeline_split(height, 0.0, 400.0, 16.0 / 9.0);
    assert!(narrow > wide + 100.0, "{narrow}");
}

#[test]
fn an_error_notice_stays_longer_than_the_rest() {
    assert!(motion::NOTICE_ERROR_HOLD > motion::NOTICE_HOLD);
    let mut state = EditorState::default();
    state.notify("Rendered master", false);
    state.notify("The project was not pushed", true);
    assert_eq!(state.notices.len(), 2);
    // Past a notice's hold and not an error's: the error stays.
    assert!(state.expire_notices(Duration::ZERO, Duration::from_secs(3600)));
    let left: Vec<&str> = state
        .notices
        .iter()
        .map(|notice| notice.text.as_str())
        .collect();
    assert_eq!(left, ["The project was not pushed"]);
    assert!(!state.expire_notices(Duration::ZERO, Duration::from_secs(3600)));
    assert!(state.expire_notices(Duration::ZERO, Duration::ZERO));
    assert!(state.notices.is_empty());
}

#[test]
fn the_palette_lists_the_recent_projects_after_the_commands() {
    let folder = tempfile::tempdir().expect("a folder");
    let harbour = folder.path().join("Harbour");
    let atlas = folder.path().join("Atlas");
    std::fs::create_dir_all(&harbour).unwrap();
    std::fs::create_dir_all(&atlas).unwrap();
    let gone = folder.path().join("Gone");
    let recent = vec![harbour.clone(), atlas.clone(), gone.clone()];

    let rows = recent_commands(&recent, Some(&harbour));
    let labels: Vec<&str> = rows
        .iter()
        .map(|(command, _)| command.label.as_str())
        .collect();
    assert_eq!(labels, ["✓ Harbour", "Atlas", "Gone"]);
    let missing: Vec<bool> = rows.iter().map(|(_, missing)| *missing).collect();
    assert_eq!(missing, [false, false, true]);
    for ((command, _), path) in rows.iter().zip(&recent) {
        assert_eq!(command.menu, RECENT_MENU);
        assert!(command.keys.is_empty());
        assert_eq!(
            recent_path(&command.id),
            Some(path.to_string_lossy().as_ref())
        );
        // "recent" finds them all, as it finds Open Recent in a menu.
        assert!(matches(command, "recent"));
        assert!(matches(command, "open recent"));
    }
    // A command of the registry is not a recent project.
    for command in catalog("kerf") {
        assert_eq!(recent_path(&command.id), None, "{}", command.id);
    }
    assert!(recent_commands(&[], None).is_empty());
}

#[test]
fn the_view_menu_has_the_mask_tools_qt_had_there() {
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

proptest! {
    #[test]
    fn a_flight_lands_inside_the_projects_row(
        x in 0f32..400.0,
        y in 0f32..800.0,
        width in 60f32..600.0,
        height in 20f32..60.0,
    ) {
        let row = Bounds::new(point(px(x), px(y)), size(px(width), px(height)));
        let landing = landing_in(row);
        let (left, top) = motion::LANDING_INSET;
        prop_assert_eq!(landing.origin, point(px(x + left), px(y + top)));
        prop_assert_eq!(f32::from(landing.size.width), motion::LANDING_WIDTH);
        prop_assert!((f32::from(landing.size.height) - (height - top * 2.0)).abs() < 1e-3);
        // Inside the row, at its start.
        prop_assert!(landing.origin.x + landing.size.width <= row.origin.x + row.size.width);
        prop_assert!(landing.origin.y + landing.size.height <= row.origin.y + row.size.height + px(1e-3));
    }
}
