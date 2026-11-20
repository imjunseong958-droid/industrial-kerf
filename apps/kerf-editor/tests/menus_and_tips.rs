//! Menus and tips, through their public functions: which way a submenu
//! opens beside its row, how tall a menu is, which rows stay lit when a
//! menu's rows change while it is open, where a tip stands for a control
//! as tall as the window and for one of several lines; the branches
//! Switch Branch offers, in the sync chip's menu and the menu bar; and the
//! folder menus' New Conversation, with another folder's conversations
//! read from where its project keeps them, each file's turns counted.

use gpui::{point, px, size, Action, Bounds, Pixels, Size};
use kerf_editor::commands::{
    self, branch_menu, menus_with_branches, MenuBranches, MenuStates, NoOtherBranches, Unavailable,
};
use kerf_editor::finish::{other_branches, NO_OTHER_BRANCHES};
use kerf_editor::sidebar::{
    conversation_summary, folder_conversations, folder_entries, other_folder_entries, reveal_words,
    Entry, SwitchBranch, CONVERSATIONS,
};
use kerf_editor::theme::{layout, text};
use kerf_editor::ui::{
    kept_path, menu_height, submenu_place, tip_anchor, tip_height, tip_origin, typed_letter,
    MenuItem, SubmenuPlace,
};
use proptest::prelude::*;

fn bounds(left: f32, top: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(left), px(top)), size(px(width), px(height)))
}

fn window(width: f32, height: f32) -> Size<Pixels> {
    size(px(width), px(height))
}

// ------------------------------------------------------------ submenus

#[test]
fn a_submenu_low_in_the_window_opens_upward_its_foot_on_the_rows() {
    // Collaborate ▸ in the app's menu over the account, in a window 1000
    // tall: its row near the foot.
    let panel = bounds(8.0, 560.0, 240.0, 420.0);
    let row = bounds(13.0, 900.0, 230.0, layout::MENU_ROW);
    let rows: Vec<MenuItem> = (0..8)
        .map(|at| MenuItem::new(format!("Row {at}")))
        .collect();
    let sub = size(px(240.0), px(menu_height(&rows)));
    assert_eq!(
        submenu_place(row, panel, sub, window(1600.0, 1000.0)),
        SubmenuPlace {
            left: false,
            up: true
        }
    );
    // High in the window it opens down, its first row level with the row.
    let row = bounds(13.0, 100.0, 230.0, layout::MENU_ROW);
    assert_eq!(
        submenu_place(row, panel, sub, window(1600.0, 1000.0)),
        SubmenuPlace::default()
    );
    // At the window's right, it opens to the left.
    let panel = bounds(1400.0, 40.0, 190.0, 300.0);
    let row = bounds(1405.0, 80.0, 180.0, layout::MENU_ROW);
    assert_eq!(
        submenu_place(row, panel, sub, window(1600.0, 1000.0)),
        SubmenuPlace {
            left: true,
            up: false
        }
    );
}

proptest! {
    /// A submenu opens where it fits; where it fits on neither side, on the
    /// side with more room.
    #[test]
    fn a_submenu_opens_where_there_is_room(
        panel_left in 0f32..1500.0,
        row_top in 0f32..960.0,
        rows in 1usize..20,
        width in 180f32..320.0,
    ) {
        let window = window(1600.0, 1000.0);
        let panel = bounds(panel_left, row_top - 40.0, width, 400.0);
        let row = bounds(panel_left + 5.0, row_top, width - 10.0, layout::MENU_ROW);
        let items: Vec<MenuItem> = (0..rows).map(|at| MenuItem::new(format!("{at}"))).collect();
        let tall = menu_height(&items);
        let sub = size(px(width), px(tall));
        let place = submenu_place(row, panel, sub, window);
        let edge = layout::TIP_EDGE;
        let right_room = 1600.0 - edge - (panel_left + width - 1.0);
        let left_room = panel_left + 1.0 - edge;
        let below = 1000.0 - edge - (row_top - 5.0);
        let above = row_top + layout::MENU_ROW + 5.0 - edge;
        if place.left {
            prop_assert!(width > right_room && left_room > right_room);
        } else {
            prop_assert!(width <= right_room || left_room <= right_room);
        }
        if place.up {
            prop_assert!(tall > below && above > below);
        } else {
            prop_assert!(tall <= below || above <= below);
        }
    }
}

#[test]
fn a_menu_is_as_tall_as_its_rows_and_the_lines_between_their_groups() {
    let items = vec![
        MenuItem::new("Insert"),
        MenuItem::new("Overwrite"),
        MenuItem::new("Replace Media…").separated(),
        MenuItem::new("Import Media…").separated(),
    ];
    // Its padding and edge, four rows, two lines.
    assert_eq!(
        menu_height(&items),
        4.0 * 2.0 + 2.0 + 4.0 * layout::MENU_ROW + 2.0 * 9.0
    );
    assert_eq!(menu_height(&[]), 10.0);
}

fn items_of(shape: &[(bool, usize)]) -> Vec<MenuItem> {
    shape
        .iter()
        .enumerate()
        .map(|(at, (enabled, children))| {
            let item = MenuItem::new(format!("{at}")).disabled(!enabled);
            if *children > 0 {
                item.submenu(
                    (0..*children)
                        .map(|c| MenuItem::new(format!("{at}.{c}")))
                        .collect(),
                )
            } else {
                item
            }
        })
        .collect()
}

proptest! {
    /// When a menu's rows change while it is open, what stays lit is a row
    /// there that can be chosen, at each level that still opens.
    #[test]
    fn what_stays_lit_is_still_there_and_can_be_chosen(
        shape in proptest::collection::vec((any::<bool>(), 0usize..4), 0..8),
        path in proptest::collection::vec(proptest::option::of(0usize..10), 1..4),
    ) {
        let items = items_of(&shape);
        let kept = kept_path(&items, &path);
        prop_assert!(!kept.is_empty() && kept.len() <= path.len());
        let mut rows = items.as_slice();
        for (level, lit) in kept.iter().enumerate() {
            if let Some(at) = lit {
                prop_assert!(*at < rows.len());
                prop_assert!(rows[*at].enabled());
                prop_assert_eq!(Some(*at), path[level]);
            }
            if level + 1 < kept.len() {
                let at = lit.expect("a level opens only under a lit row");
                rows = shape_children(&items, &kept[..=level]);
                prop_assert!(!rows.is_empty(), "{at}");
            }
        }
    }
}

/// The rows under the lit rows of `path`, from the top.
fn shape_children<'a>(items: &'a [MenuItem], path: &[Option<usize>]) -> &'a [MenuItem] {
    let mut rows = items;
    for lit in path {
        let Some(item) = lit.and_then(|at| rows.get(at)) else {
            return &[];
        };
        rows = item.rows();
    }
    rows
}

#[test]
fn a_row_gone_or_greyed_is_lit_no_more_and_takes_its_submenu_with_it() {
    let before = items_of(&[(true, 0), (true, 2), (true, 0)]);
    assert_eq!(
        kept_path(&before, &[Some(1), Some(1)]),
        vec![Some(1), Some(1)]
    );
    let greyed = items_of(&[(true, 0), (false, 2), (true, 0)]);
    assert_eq!(kept_path(&greyed, &[Some(1), Some(1)]), vec![None]);
    let fewer = items_of(&[(true, 0)]);
    assert_eq!(kept_path(&fewer, &[Some(2)]), vec![None]);
}

#[test]
fn a_letter_typed_is_one_letter_or_figure() {
    assert_eq!(typed_letter("c"), Some('c'));
    assert_eq!(typed_letter("7"), Some('7'));
    assert_eq!(typed_letter("é"), Some('é'));
    assert_eq!(typed_letter("backspace"), None);
    assert_eq!(typed_letter("-"), None);
    assert_eq!(typed_letter(""), None);
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

// ------------------------------------------------------------ folders

#[test]
fn a_folders_menu_starts_a_new_conversation_in_it() {
    let root = tempfile::tempdir().unwrap();
    let open = root.path().join("harbour");
    let other = root.path().join("trailer");
    std::fs::create_dir_all(&open).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let entries = folder_entries(std::slice::from_ref(&other), Some(&open));
    assert_eq!(entries[0].words, "New Conversation");
    assert_eq!(entries[0].entry, Entry::NewConversation(open.clone()));
    // The folders follow, after a line.
    assert!(entries[1].separated && entries[1].entry == Entry::Open(open.clone()));
    // With no folder open there is none to start one in.
    assert!(!folder_entries(&[], None)
        .iter()
        .any(|entry| matches!(entry.entry, Entry::NewConversation(_))));
    // Another folder's menu, as Qt's.
    let words: Vec<String> = other_folder_entries(&other)
        .into_iter()
        .map(|entry| entry.words)
        .collect();
    assert_eq!(
        words,
        [
            "New Conversation",
            "Open Project",
            reveal_words(),
            "Copy Path",
            "Remove from Sidebar"
        ]
    );
    assert_eq!(
        other_folder_entries(&other)[0].entry,
        Entry::NewConversation(other.clone())
    );
}

fn conversation(id: &str, title: &str, updated: &str, archived: bool, items: usize) -> String {
    let items: Vec<serde_json::Value> = (0..items)
        .map(|at| serde_json::json!({"id": format!("i{at}"), "kind": "you", "text": "Cut it", "at": updated}))
        .collect();
    serde_json::json!({
        "id": id, "title": title, "created": "2026-11-01T10:00:00Z", "updated": updated,
        "archived": archived, "items": items,
    })
    .to_string()
}

#[test]
fn another_folders_conversations_are_read_where_its_project_keeps_them() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("trailer");
    let kept = folder.join(CONVERSATIONS);
    std::fs::create_dir_all(&kept).unwrap();
    let write = |name: &str, text: &str| std::fs::write(kept.join(name), text).unwrap();
    write(
        "c-old.json",
        &conversation(
            "c-old",
            "Tighten the open",
            "2026-11-02T10:00:00Z",
            false,
            2,
        ),
    );
    write(
        "c-new.json",
        &conversation(
            "c-new",
            "Music under the end",
            "2026-11-19T08:00:00Z",
            false,
            4,
        ),
    );
    write(
        "c-gone.json",
        &conversation("c-gone", "Old cut", "2026-11-20T08:00:00Z", true, 1),
    );
    // Not a conversation's: an id the bridge never makes, and no JSON.
    write(
        "stray.json",
        &conversation("../../etc", "Stray", "2026-11-20T09:00:00Z", false, 0),
    );
    write("broken.json", "{ not json");
    write("notes.txt", "not a conversation");
    let listed = folder_conversations(&folder);
    let ids: Vec<&str> = listed.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["c-new", "c-old"]);
    assert_eq!(listed[0].title, "Music under the end");
    assert_eq!(listed[0].count, 4);
    // A folder with none yet, or none at all, lists none.
    assert!(folder_conversations(&root.path().join("empty")).is_empty());
    let summary = conversation_summary(&conversation("c-1", "", "x", false, 0)).unwrap();
    assert_eq!(summary.label(), "New conversation");
}

/// What the sidebar read of a conversation's file before it counted the
/// turns without keeping them: the whole file as JSON.
fn summary_as_read_whole(text: &str) -> Option<(String, String, String, String, bool, u32)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let id = value["id"].as_str()?;
    let valid = id.len() <= 80
        && id.starts_with("c-")
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if !valid {
        return None;
    }
    let text_of = |name: &str| value[name].as_str().unwrap_or_default().to_owned();
    Some((
        id.to_owned(),
        text_of("title"),
        text_of("created"),
        text_of("updated"),
        value["archived"].as_bool().unwrap_or(false),
        value["items"]
            .as_array()
            .map_or(0, |items| items.len() as u32),
    ))
}

/// Any JSON, a few levels deep.
fn any_json() -> impl Strategy<Value = serde_json::Value> {
    use serde_json::Value;
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        (-1e9f64..1e9).prop_map(Value::from),
        prop_oneof![
            Just("c-1".to_owned()),
            "[a-z0-9 -]{0,8}",
            "c-[A-Za-z0-9_-]{0,6}"
        ]
        .prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            proptest::collection::btree_map("[a-z]{1,3}", inner, 0..4)
                .prop_map(|map| Value::Object(map.into_iter().collect())),
        ]
    })
}

proptest! {
    /// A conversation's file reads as it did whole, its turns only counted:
    /// whatever its fields hold, and whatever else it holds.
    #[test]
    fn a_conversations_file_reads_as_it_did_whole(
        fields in proptest::collection::btree_map(
            prop_oneof![
                Just("id".to_owned()),
                Just("title".to_owned()),
                Just("created".to_owned()),
                Just("updated".to_owned()),
                Just("archived".to_owned()),
                Just("items".to_owned()),
                "[a-z]{1,6}",
            ],
            any_json(),
            0..8,
        ),
        id in proptest::option::of("c-[A-Za-z0-9_-]{0,6}"),
        whole in any_json(),
        object in any::<bool>(),
    ) {
        let text = if object {
            let mut fields: serde_json::Map<String, serde_json::Value> = fields.into_iter().collect();
            if let Some(id) = id {
                fields.insert("id".to_owned(), id.into());
            }
            serde_json::Value::Object(fields).to_string()
        } else {
            whole.to_string()
        };
        let read = conversation_summary(&text).map(|summary| {
            (summary.id, summary.title, summary.created, summary.updated, summary.archived, summary.count)
        });
        prop_assert_eq!(read, summary_as_read_whole(&text), "{}", text);
    }
}
