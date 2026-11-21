//! The window, the picture and the timeline, through the public API: where
//! a window opens and the title bar it has, the menu bar's keymaps, how
//! long ago the project was edited on the person's calendar, the preview's
//! version and why it needs rendering, the clips a change reel lifts, and
//! the tracks' roomy height.

use gpui::{point, px, size, Action, Bounds, WindowBounds};
use jiff::tz::TimeZone;
use kerf_editor::app::{lead_past, titlebar, window_buttons_at, window_place};
use kerf_editor::commands::{keymap_menu, menus_with};
use kerf_editor::model::{Snapshot, TrackKind};
use kerf_editor::prefs::Keymap;
use kerf_editor::sidebar::ChooseKeymap;
use kerf_editor::stage::{preview_chip_words, previewed_version};
use kerf_editor::store::Preview;
use kerf_editor::theme::{layout, motion};
use kerf_editor::timeline::{
    default_height, watch_look, watched_clips, ClipWatch, TrackHeights, WatchLook,
};
use kerf_editor::workspace::{edited_ago_in, local_day, toolbar_places, ToolbarPlaces};
use proptest::prelude::*;
use serde_json::json;
use std::cell::Cell;
use std::collections::HashSet;
use std::time::{Duration, Instant};

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
fn a_first_window_opens_at_qts_size_in_the_middle_where_it_fits() {
    // A 1080p screen: 1500 × 940 in its middle, windowed.
    let wide = screen(0.0, 0.0, 1920.0, 1080.0);
    assert_eq!(
        window_place(None, None, Some(wide)),
        WindowBounds::Windowed(screen(210.0, 70.0, 1500.0, 940.0))
    );
    // A screen to the right of another: in its own middle.
    let beside = screen(1920.0, 0.0, 2560.0, 1440.0);
    assert_eq!(
        window_place(None, None, Some(beside)),
        WindowBounds::Windowed(screen(1920.0 + 530.0, 250.0, 1500.0, 940.0))
    );
    // No screen known: at the origin.
    assert_eq!(
        window_place(None, None, None),
        WindowBounds::Windowed(screen(0.0, 0.0, 1500.0, 940.0))
    );
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
    // Too short only: still filled, as wide as Qt's where that fits.
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
    /// on the screen; windowed only at Qt's size with the room kept, else
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

// ------------------------------------------------------------- the menu bar

fn rows(menu: &gpui::Menu) -> Vec<(String, Box<dyn Action>)> {
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
fn edit_has_keyboard_shortcuts_with_the_keymap_in_use_ticked() {
    for keymap in Keymap::ALL {
        let menus = menus_with(keymap.wire(), &[], None);
        let edit = menus
            .iter()
            .find(|menu| menu.name == "Edit")
            .expect("an Edit menu");
        // At Edit's foot, after a line.
        let last = edit.items.len() - 1;
        assert!(matches!(edit.items[last - 1], gpui::MenuItem::Separator));
        let gpui::MenuItem::Submenu(shortcuts) = &edit.items[last] else {
            panic!("Keyboard Shortcuts is a submenu");
        };
        assert_eq!(shortcuts.name, "Keyboard Shortcuts");
        let rows = rows(shortcuts);
        assert_eq!(rows.len(), Keymap::ALL.len());
        for ((name, action), choice) in rows.iter().zip(Keymap::ALL) {
            // By the names Settings gives them; the one in use ticked.
            let expected = if choice == keymap {
                format!("✓ {}", choice.label())
            } else {
                choice.label().to_owned()
            };
            assert_eq!(*name, expected);
            assert!(action.partial_eq(&ChooseKeymap { keymap: choice }));
        }
    }
    let names: Vec<String> = rows(&keymap_menu("fcp"))
        .into_iter()
        .map(|row| row.0)
        .collect();
    assert_eq!(names, ["Kerf", "Premiere Pro", "✓ Final Cut Pro"]);
    // No other menu has it.
    let menus = menus_with("kerf", &[], None);
    let elsewhere = menus.iter().filter(|menu| menu.name != "Edit").any(|menu| {
        menu.items.iter().any(
            |item| matches!(item, gpui::MenuItem::Submenu(sub) if sub.name == "Keyboard Shortcuts"),
        )
    });
    assert!(!elsewhere);
}

// ----------------------------------------------------- edited, how long ago

/// Seconds since 1970 of `text` (RFC 3339).
fn at(text: &str) -> i64 {
    text.parse::<jiff::Timestamp>().unwrap().as_second()
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

// --------------------------------------------------------------- the preview

fn proposals() -> Snapshot {
    Snapshot::from_value(&json!({
        "timeline": {"fps": 24.0, "tracks": []},
        "proposals": [
            {"proposalId": "p3", "status": "pending", "version": 3,
             "versions": [{"version": 1, "proposalId": "p1"}, {"version": 2, "proposalId": "p2"},
                          {"version": 3, "proposalId": "p3"}]},
            {"proposalId": "q", "status": "pending", "version": 1}
        ]
    }))
}

#[test]
fn the_preview_chip_names_the_version_that_plays() {
    let snapshot = proposals();
    let proposals = &snapshot.proposals;
    assert_eq!(previewed_version(proposals, "p2"), Some(2));
    assert_eq!(previewed_version(proposals, "p3"), Some(3));
    // A proposal with no versions listed: its own.
    assert_eq!(previewed_version(proposals, "q"), Some(1));
    assert_eq!(previewed_version(proposals, "gone"), None);
    assert_eq!(preview_chip_words(Some(2)), "Previewing proposal v2");
    assert_eq!(preview_chip_words(None), "Previewing proposal");
}

#[test]
fn a_preview_to_render_says_why() {
    let required = |message: Option<&str>| Preview {
        status: "required".into(),
        message: message.map(str::to_owned),
        ..Preview::default()
    };
    assert_eq!(
        required(Some("Blur needs a render")).reason().as_deref(),
        Some("Blur needs a render")
    );
    // With no reason given, Qt's words.
    assert_eq!(
        required(None).reason().as_deref(),
        Some("Rendered preview required")
    );
    assert_eq!(
        required(Some("  ")).reason().as_deref(),
        Some("Rendered preview required")
    );
    let ready = Preview {
        status: "ready".into(),
        ..Preview::default()
    };
    assert_eq!(ready.reason(), None);
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

#[test]
fn every_track_starts_at_the_roomy_height_qt_s_window_drew() {
    // Qt's window set roomy lanes: 40 for every track, picture, sound and
    // subtitles alike.
    assert_eq!(layout::TRACK_LANE, 40.0);
    let heights = TrackHeights::default();
    for kind in [TrackKind::Video, TrackKind::Audio, TrackKind::Subtitle] {
        assert_eq!(default_height(kind), layout::TRACK_LANE);
        assert_eq!(heights.height("T", kind), layout::TRACK_LANE);
    }
    // Medium for every track, and Reset, go back to it.
    let mut heights = TrackHeights::default();
    heights.set_all(Some(64.0));
    heights.set("A1", Some(20.0));
    heights.set_all(None);
    assert_eq!(heights.height("A1", TrackKind::Audio), layout::TRACK_LANE);
    assert_eq!(heights, TrackHeights::default());
}
