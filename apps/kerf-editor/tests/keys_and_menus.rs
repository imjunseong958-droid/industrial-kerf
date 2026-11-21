//! Keys and menus, through their public functions: which Esc goes first
//! (an open menu, a preview rendering, the Media band), what a key does
//! while one of the kit's menus is open, the macOS application menu, the
//! mask tools' ticks and greys in the menu bar and the app's menu, Ask
//! Kerf…'s reason wherever it is listed, View ▸ Sync's failure, and what a
//! failed change to a conversation says.

use gpui::{Keystroke, MenuItem};
use kerf_editor::commands::{
    self, app_menu, app_menu_bindings, mask_tool_id, menus_with_states, HideApp, HideOtherApps,
    MenuStates, ShowAllApps, Unavailable, NO_MASK_CLIP,
};
use kerf_editor::prefs::Keymap;
use kerf_editor::sidebar::{
    app_rows, sync_problem, with_sync_failure, AppDoes, AppMenuInput, AppRow, SYNC_FAILURE,
};
use kerf_editor::stage::MaskTool;
use kerf_editor::ui::{menu_key, MenuKey};
use kerf_editor::workspace::{
    conversation_error, escape_goes, render_card_shows, render_stop_holds, EscapeAt, EscapeGoes,
    RenderCard,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

// ------------------------------------------------------------ Esc

fn every_escape() -> Vec<EscapeAt> {
    (0u32..1 << 7)
        .map(|bits| {
            let on = |bit: u32| bits & (1 << bit) != 0;
            EscapeAt {
                menu_has_keys: on(0),
                menu_open: on(1),
                in_field: on(2),
                in_dialog: on(3),
                on_picture: on(4),
                rendering: on(5),
                band_open: on(6),
            }
        })
        .collect()
}

#[test]
fn esc_goes_to_a_menu_first_then_a_render_then_the_band_then_the_rest() {
    for at in every_escape() {
        let goes = escape_goes(at);
        let menu = at.menu_has_keys || at.menu_open;
        let held = at.in_field || at.in_dialog;
        let expected = if at.menu_has_keys {
            // The menu with the keys closes itself (its deepest submenu first).
            EscapeGoes::On
        } else if at.menu_open {
            EscapeGoes::CloseMenus
        } else if held {
            EscapeGoes::On
        } else if at.rendering {
            EscapeGoes::StopRender
        } else if at.band_open && !at.on_picture {
            EscapeGoes::PutBandAway
        } else {
            EscapeGoes::On
        };
        assert_eq!(goes, expected, "{at:?}");
        // A render never stops, nor the band closes, while a menu is open
        // or someone types in a field or a dialog.
        if menu || held {
            assert!(
                !matches!(goes, EscapeGoes::StopRender | EscapeGoes::PutBandAway),
                "{at:?}"
            );
        }
        // The band closes only once nothing renders.
        if goes == EscapeGoes::PutBandAway {
            assert!(!at.rendering && at.band_open && !at.on_picture, "{at:?}");
        }
    }
}

#[test]
fn esc_on_the_timeline_stops_a_render_before_rejecting_or_letting_a_clip_go() {
    // The timeline has the keys (nothing of the window's own takes Esc),
    // a proposal's preview renders, the band is open: the render stops.
    let timeline = EscapeAt {
        rendering: true,
        band_open: true,
        ..EscapeAt::default()
    };
    assert_eq!(escape_goes(timeline), EscapeGoes::StopRender);
    // Then the band.
    let rendered = EscapeAt {
        rendering: false,
        ..timeline
    };
    assert_eq!(escape_goes(rendered), EscapeGoes::PutBandAway);
    // Then the timeline's own (Reject Proposal, letting the clip go).
    assert_eq!(escape_goes(EscapeAt::default()), EscapeGoes::On);
    // From the picture, the render stops all the same; the band waits for
    // what the picture leaves of Esc.
    let picture = EscapeAt {
        on_picture: true,
        ..timeline
    };
    assert_eq!(escape_goes(picture), EscapeGoes::StopRender);
    assert_eq!(
        escape_goes(EscapeAt {
            rendering: false,
            ..picture
        }),
        EscapeGoes::On
    );
    // Typing, or in Settings, Esc is the field's or the dialog's.
    for held in [
        EscapeAt {
            in_field: true,
            ..timeline
        },
        EscapeAt {
            in_dialog: true,
            ..timeline
        },
    ] {
        assert_eq!(escape_goes(held), EscapeGoes::On);
    }
}

#[test]
fn esc_stops_a_render_only_while_its_card_shows() {
    let shown = RenderCard {
        rendering: true,
        picture_shown: true,
        ..RenderCard::default()
    };
    assert!(render_card_shows(shown));
    // No card while Kerf builds its cut, while a change reel plays (or a
    // first cut is cut), once stopped here, or with the picture away: Esc
    // goes on to the band and the timeline, and no render stops unseen.
    for hidden in [
        RenderCard {
            kerf_building: true,
            ..shown
        },
        RenderCard {
            reel: true,
            ..shown
        },
        RenderCard {
            stopped: true,
            ..shown
        },
        RenderCard {
            picture_shown: false,
            ..shown
        },
        RenderCard {
            rendering: false,
            ..shown
        },
    ] {
        assert!(!render_card_shows(hidden), "{hidden:?}");
        let at = EscapeAt {
            rendering: render_card_shows(hidden),
            band_open: true,
            ..EscapeAt::default()
        };
        assert_eq!(escape_goes(at), EscapeGoes::PutBandAway, "{hidden:?}");
        assert_eq!(
            escape_goes(EscapeAt {
                band_open: false,
                ..at
            }),
            EscapeGoes::On,
            "{hidden:?}"
        );
    }
}

#[test]
fn esc_in_the_chips_question_is_the_questions() {
    // Publish ▸ This computer, the keys on the question (not its field),
    // the band open and a render's card showing: the question's own Close
    // has Esc, as Qt's modal dialog had it.
    let question = EscapeAt {
        in_dialog: true,
        rendering: true,
        band_open: true,
        ..EscapeAt::default()
    };
    assert_eq!(escape_goes(question), EscapeGoes::On);
}

proptest! {
    /// A render stopped here keeps its card done with while that render
    /// runs on, and no longer: any other status ends it, another
    /// proposal's render takes over, and other events leave it be.
    #[test]
    fn a_stopped_render_stays_stopped_until_its_run_ends(
        stopped in "[a-c]{0,2}",
        proposal in "[a-c]{0,2}",
        status in prop::sample::select(vec!["running", "done", "stopped", "failed", "required", ""]),
        kind in prop::sample::select(vec!["preview", "state", "agent", "progress"]),
    ) {
        let event = json!({"type": kind, "status": status, "proposalId": proposal});
        let holds = render_stop_holds(Some(&stopped), &event);
        if kind != "preview" {
            prop_assert!(holds);
        } else {
            prop_assert_eq!(holds, status == "running" && proposal == stopped);
        }
        // Nothing stopped, nothing held.
        prop_assert!(!render_stop_holds(None, &event));
    }
}

// ------------------------------------------------------------ the kit's menus

fn key(text: &str) -> MenuKey {
    menu_key(&Keystroke::parse(text).unwrap())
}

#[test]
fn an_open_menu_holds_every_key_it_does_not_use() {
    // The window's shortcuts do nothing under it: the palette, Undo, Quit.
    for held in [
        "secondary-shift-p",
        "ctrl-shift-p",
        "cmd-z",
        "secondary-q",
        "alt-up",
        "ctrl-j",
        "alt-a",
        "backspace",
        "delete",
        "f2",
        "-",
    ] {
        assert_eq!(key(held), MenuKey::Held, "{held}");
    }
    // Its own keys go on to it; a letter goes to its row.
    for own in [
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "enter",
        "space",
        "escape",
        "tab",
        "shift-tab",
    ] {
        assert_eq!(key(own), MenuKey::Own, "{own}");
    }
    assert_eq!(key("r"), MenuKey::Letter('r'));
    assert_eq!(key("shift-r"), MenuKey::Letter('r'));
    assert_eq!(key("7"), MenuKey::Letter('7'));
}

proptest! {
    #[test]
    fn a_key_with_ctrl_cmd_or_alt_is_always_held(
        letter in "[a-z0-9]",
        control in any::<bool>(),
        alt in any::<bool>(),
        platform in any::<bool>(),
        shift in any::<bool>(),
    ) {
        prop_assume!(control || alt || platform);
        let mut keystroke = Keystroke::parse(&letter).unwrap();
        keystroke.modifiers.control = control;
        keystroke.modifiers.alt = alt;
        keystroke.modifiers.platform = platform;
        keystroke.modifiers.shift = shift;
        prop_assert_eq!(menu_key(&keystroke), MenuKey::Held);
        // Without them, the same letter jumps to its row.
        keystroke.modifiers.control = false;
        keystroke.modifiers.alt = false;
        keystroke.modifiers.platform = false;
        let typed = letter.chars().next().unwrap();
        prop_assert_eq!(menu_key(&keystroke), MenuKey::Letter(typed));
    }
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
    let menus = commands::menus("kerf");
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
        keymap: Keymap::Kerf,
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

fn find<'a>(rows: &'a [AppRow], words: &str) -> &'a AppRow {
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
        let rows = app_rows(&input(&commands, &menus, &states, &checked, None, menu_bar));
        let mut views = vec![find(&rows, "View")];
        if !menu_bar {
            views.extend(rows.iter().filter(|row| row.words == "View").skip(1));
        }
        for view in views {
            for tool in MaskTool::ALL {
                let row = find(&view.rows, tool.label());
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
    let rows = app_rows(&input(&commands, &menus, &states, &checked, None, false));
    for view in rows.iter().filter(|row| row.words == "View") {
        assert!(find(&view.rows, "Ellipse mask").checked);
        assert!(!find(&view.rows, "Rectangle mask").checked);
        assert!(!find(&view.rows, "Pen mask").disabled);
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
    let rows = app_rows(&input(&commands, &menus, &states, &checked, None, false));
    let clip = find(&rows, "Clip");
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
    let rows = app_rows(&input(&commands, &menus, &busy, &checked, None, false));
    let ask = find(&rows, "Clip")
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
    // An unfinished merge is said first, as Qt's.
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
    let rows = app_rows(&input(
        &commands,
        &[],
        &states,
        &checked,
        Some(&failed),
        true,
    ));
    let sync = find(&find(&rows, "View").rows, "Sync");
    assert_eq!(sync.rows[1].words, "Sync failed: could not reach origin");
    assert!(sync.rows[1].disabled);
    assert_eq!(sync.rows[1].does, AppDoes::Nothing);
    let rows = app_rows(&input(
        &commands,
        &[],
        &states,
        &checked,
        Some(&synced),
        true,
    ));
    let sync = find(&find(&rows, "View").rows, "Sync");
    assert!(!sync
        .rows
        .iter()
        .any(|row| row.words.starts_with("Sync failed")));
}

// ------------------------------------------------------------ conversations

#[test]
fn a_failed_change_to_a_conversation_says_so() {
    assert_eq!(
        conversation_error("no conversation `k1`"),
        "Couldn't change the conversation · no conversation `k1`"
    );
}
