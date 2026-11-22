//! The assembly: binding a keymap (the registry's shortcuts, the kit's
//! keys and each view's own) and building the menu bar from the registry.
//! Both name the views whose actions they reach; the views reach them
//! through [`Assembly`].

use editor_session::commands::{
    catalog, recent_rows, Assembly, ClearRecent, Command, Escape, HideApp, HideOtherApps,
    MenuBranches, MenuStates, NoOtherBranches, NoRecent, OpenRecent, RecentMissing, RunCommand,
    ShowAllApps, Unavailable, TIMELINE, WORKSPACE,
};
use editor_stage::stage::mask_tool_id;
use gpui::{App, KeyBinding, NoAction, SharedString};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The application menu (macOS): About Kerf; Settings…; Services ▸; Hide
/// Industrial Kerf, Hide Others and Show All; Quit: the items and order a
/// Mac application's own menu has, so they are where people look for them.
pub fn app_menu() -> gpui::Menu {
    gpui::Menu {
        name: "Industrial Kerf".into(),
        items: vec![
            gpui::MenuItem::action("About Kerf", RunCommand::new("help.about")),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Settings…", RunCommand::new("app.preferences")),
            gpui::MenuItem::separator(),
            gpui::MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Hide Industrial Kerf", HideApp),
            gpui::MenuItem::action("Hide Others", HideOtherApps),
            gpui::MenuItem::action("Show All", ShowAllApps),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Quit Industrial Kerf", RunCommand::new("app.quit")),
        ],
    }
}

/// The keys of the application menu's own rows on macOS, whatever the
/// keymap says: ⌘H hides the editor, ⌥⌘H the other applications. Elsewhere
/// the system has no such rows, and none are bound.
pub fn app_menu_bindings() -> Vec<KeyBinding> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    vec![
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("alt-cmd-h", HideOtherApps, None),
    ]
}

/// Whether the application menu's own rows are answered already.
struct AppMenuAnswered;

impl gpui::Global for AppMenuAnswered {}

/// Answers the application menu's own rows, in every window and with none
/// open: Hide, Hide Others and Show All, by the system. Once is enough.
pub fn answer_app_menu(cx: &mut App) {
    if cx.has_global::<AppMenuAnswered>() {
        return;
    }
    cx.set_global(AppMenuAnswered);
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
}

/// What takes out mask tool `tool` (or puts it down) from a menu.
pub fn mask_tool_action(tool: editor_stage::stage::MaskTool) -> Box<dyn gpui::Action> {
    match tool {
        editor_stage::stage::MaskTool::Rectangle => Box::new(editor_stage::stage::RectangleMask),
        editor_stage::stage::MaskTool::Ellipse => Box::new(editor_stage::stage::EllipseMask),
        editor_stage::stage::MaskTool::Pen => Box::new(editor_stage::stage::PenMask),
    }
}

/// Collaborate ▸ Switch Branch as a submenu: each other
/// branch, switching to it; "No Other Branches" (greyed)
/// when there is none. Held back, each branch is greyed.
pub fn branch_menu(branches: &MenuBranches) -> gpui::Menu {
    let mut items: Vec<gpui::MenuItem> = branches
        .others
        .iter()
        .map(|branch| {
            if branches.blocked {
                gpui::MenuItem::action(branch.clone(), Unavailable)
            } else {
                gpui::MenuItem::action(
                    branch.clone(),
                    editor_sidebar::sidebar::SwitchBranch {
                        name: branch.clone().into(),
                    },
                )
            }
        })
        .collect();
    if items.is_empty() {
        items.push(gpui::MenuItem::action(
            editor_finish::finish::NO_OTHER_BRANCHES,
            NoOtherBranches,
        ));
    }
    gpui::Menu {
        name: "Switch Branch".into(),
        items,
    }
}

/// Edit ▸ Keyboard Shortcuts in the menu bar: one row for each
/// keymap, by the name Settings gives it, the one in use (`keymap`,
/// as the registry names it) ticked ("✓ ", since a menu bar's rows here
/// carry no tick of their own). Choosing one saves it, as Settings does
/// (`sidebar::ChooseKeymap`, which the window carries out), and the menus
/// are set again with it.
pub fn keymap_menu(keymap: &str) -> gpui::Menu {
    let items = editor_state::prefs::Keymap::ALL
        .into_iter()
        .map(|choice| {
            let name = if choice.wire() == keymap {
                format!("✓ {}", choice.label())
            } else {
                choice.label().to_owned()
            };
            gpui::MenuItem::action(
                name,
                editor_sidebar::sidebar::ChooseKeymap { keymap: choice },
            )
        })
        .collect();
    gpui::Menu {
        name: "Keyboard Shortcuts".into(),
        items,
    }
}

/// What a menu row of a command says: its label, ticked ("✓ ") while it
/// is on, since a menu bar's rows here carry no tick of their own.
pub fn menu_label(command: &Command, checked: bool) -> String {
    if command.checkable && checked {
        format!("✓ {}", command.label)
    } else {
        command.label.clone()
    }
}

/// File ▸ Open Recent, as a menu: the rows of [`recent_rows`] ("✓" before
/// the one open here, since a menu bar's rows here carry no check of their
/// own), "No Recent Projects" when there are none, then Clear Menu.
pub fn recent_menu(recent: &[PathBuf], current: Option<&Path>) -> gpui::Menu {
    let rows = recent_rows(recent, current);
    let mut items: Vec<gpui::MenuItem> = rows
        .iter()
        .map(|row| {
            let name = if row.current {
                format!("✓ {}", row.name)
            } else {
                row.name.clone()
            };
            if row.missing {
                gpui::MenuItem::action(name, RecentMissing)
            } else {
                gpui::MenuItem::action(
                    name,
                    OpenRecent {
                        path: row.path.to_string_lossy().into_owned().into(),
                    },
                )
            }
        })
        .collect();
    if items.is_empty() {
        items.push(gpui::MenuItem::action("No Recent Projects", NoRecent));
    }
    items.push(gpui::MenuItem::separator());
    // With nothing to clear, Clear Menu is greyed as the empty row is.
    items.push(if rows.is_empty() {
        gpui::MenuItem::action("Clear Menu", NoRecent)
    } else {
        gpui::MenuItem::action("Clear Menu", ClearRecent)
    });
    gpui::Menu {
        name: "Open Recent".into(),
        items,
    }
}

/// Whether a keystroke has no modifier but Shift: typing it in a field
/// types it, so it is unbound there.
fn plain(keystroke: &str) -> bool {
    let first = keystroke.split(' ').next().unwrap_or(keystroke);
    !["secondary-", "ctrl-", "alt-", "super-", "cmd-"]
        .iter()
        .any(|modifier| first.contains(modifier))
}

/// Binds every command's shortcuts in `keymap`. The first time, it
/// sets itself and [`menus_with_branches`] as the [`Assembly`].
pub fn bind(keymap: &str, cx: &mut App) {
    if !cx.has_global::<Assembly>() {
        cx.set_global(Assembly {
            bind,
            menus: menus_with_branches,
        });
    }
    answer_app_menu(cx);
    cx.clear_key_bindings();
    cx.bind_keys(editor_ui::input::key_bindings());
    cx.bind_keys(editor_chat::chat::key_bindings());
    cx.bind_keys(editor_ui::ui::key_bindings());
    editor_ui::ui::follow_tab(WORKSPACE, cx);
    let mut bindings = Vec::new();
    for command in catalog(keymap) {
        let id: SharedString = command.id.clone().into();
        for shortcut in &command.keys {
            let Some(keystroke) = editor_state::shortcuts::keystroke(shortcut) else {
                continue;
            };
            if keystroke
                .split_whitespace()
                .any(|part| gpui::Keystroke::parse(part).is_err())
            {
                continue;
            }
            let context = if command.scope == "timeline" {
                TIMELINE
            } else {
                WORKSPACE
            };
            bindings.push(KeyBinding::new(
                &keystroke,
                RunCommand { id: id.clone() },
                Some(context),
            ));
            if plain(&keystroke) {
                bindings.push(KeyBinding::new(
                    &keystroke,
                    NoAction,
                    Some(editor_ui::input::CONTEXT),
                ));
            }
        }
    }
    // What the window answers to whatever the keymap says.
    bindings.push(KeyBinding::new(
        "secondary-,",
        RunCommand::new("app.preferences"),
        Some(WORKSPACE),
    ));
    bindings.push(KeyBinding::new(
        "secondary-q",
        RunCommand::new("app.quit"),
        None,
    ));
    bindings.push(KeyBinding::new(
        "secondary-shift-p",
        editor_session::palette::OpenPalette,
        Some(WORKSPACE),
    ));
    bindings.push(KeyBinding::new("escape", Escape, Some(WORKSPACE)));
    // The application menu's Hide and Hide Others (macOS).
    bindings.extend(app_menu_bindings());
    bindings.push(KeyBinding::new(
        "up",
        editor_session::palette::Previous,
        Some(editor_session::palette::CONTEXT),
    ));
    bindings.push(KeyBinding::new(
        "down",
        editor_session::palette::Next,
        Some(editor_session::palette::CONTEXT),
    ));
    cx.bind_keys(bindings);
    // What the views bind of their own, which binding a keymap cleared: the
    // picture's mask tools and its Delete, Esc and Return; the sidebar's
    // Tab, menus and rows.
    editor_stage::stage::bind_keys(cx);
    editor_sidebar::sidebar::bind_keys(cx);
    // Esc puts away what is open over the window before the window's own Esc
    // (Reject Proposal) has it: their contexts are the deeper ones.
    editor_launcher::settings::bind_close(cx);
    editor_finish::export::bind_close(cx);
    // A shot's graph's Esc and its canvas's keys, should a keymap be bound
    // while one is open.
    editor_making::making::bind_keys(cx);
    let recent = editor_state::prefs::Prefs::load().recent;
    cx.set_menus(menus_with(keymap, &recent, None));
}

/// The registry's menus, for a menu bar (macOS draws it; elsewhere the
/// command palette and the shortcuts reach the same commands), with no
/// recent project in File ▸ Open Recent.
pub fn menus(keymap: &str) -> Vec<gpui::Menu> {
    menus_with(keymap, &[], None)
}

/// [`menus`], File ▸ Open Recent following Open Project with `recent`
/// ([`recent_menu`], `current` checked).
pub fn menus_with(keymap: &str, recent: &[PathBuf], current: Option<&Path>) -> Vec<gpui::Menu> {
    menus_with_states(keymap, recent, current, &MenuStates::default())
}

/// [`menus_with`], each row of the registry's greyed while it cannot run
/// and ticked while it is on, as `states` says ([`menu_label`]).
pub fn menus_with_states(
    keymap: &str,
    recent: &[PathBuf],
    current: Option<&Path>,
    states: &MenuStates,
) -> Vec<gpui::Menu> {
    menus_with_branches(keymap, recent, current, states, &MenuBranches::default())
}

/// [`menus_with_states`], Collaborate ▸ Switch Branch a submenu of
/// `branches` ([`branch_menu`]).
pub fn menus_with_branches(
    keymap: &str,
    recent: &[PathBuf],
    current: Option<&Path>,
    states: &MenuStates,
    branches: &MenuBranches,
) -> Vec<gpui::Menu> {
    let commands = catalog(keymap);
    let order: Vec<String> = kerf_bridge::commands_catalog(keymap)["menus"]
        .as_array()
        .map(|menus| {
            menus
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut menus = vec![app_menu()];
    for menu in order {
        let mut items = Vec::new();
        let mut group = None;
        for command in commands.iter().filter(|command| command.menu == menu) {
            if matches!(
                command.id.as_str(),
                "app.quit" | "app.preferences" | "help.about"
            ) {
                continue;
            }
            if group.is_some_and(|group| group != command.group) {
                items.push(gpui::MenuItem::separator());
            }
            group = Some(command.group);
            // Switch Branch needs a branch: it opens on the others.
            if command.id == "collab.switchBranch" {
                items.push(gpui::MenuItem::submenu(branch_menu(branches)));
                continue;
            }
            let label = menu_label(command, states.checked.contains(&command.id));
            items.push(if states.unavailable.contains(&command.id) {
                gpui::MenuItem::action(label, Unavailable)
            } else {
                gpui::MenuItem::action(
                    label,
                    RunCommand {
                        id: command.id.clone().into(),
                    },
                )
            });
            if command.id == "file.openProject" {
                items.push(gpui::MenuItem::submenu(recent_menu(recent, current)));
            }
        }
        // The keymaps to choose from, at Edit's foot.
        if menu == "Edit" && !items.is_empty() {
            items.push(gpui::MenuItem::separator());
            items.push(gpui::MenuItem::submenu(keymap_menu(keymap)));
        }
        // The picture's mask tools, which the registry does not carry, in the
        // View menu (their keys, ⇧R ⇧E ⇧P, the stage binds):
        // the one out ticked, all greyed until a picture clip the picture
        // can place is chosen ([`mask_tool_id`]).
        if menu == "View" && !items.is_empty() {
            items.push(gpui::MenuItem::separator());
            for tool in editor_stage::stage::MaskTool::ALL {
                let id = mask_tool_id(tool);
                let name = if states.checked.contains(id) {
                    format!("✓ {}", tool.label())
                } else {
                    tool.label().to_owned()
                };
                items.push(if states.unavailable.contains(id) {
                    gpui::MenuItem::action(name, Unavailable)
                } else {
                    gpui::MenuItem::Action {
                        name: name.into(),
                        action: mask_tool_action(tool),
                        os_action: None,
                    }
                });
            }
        }
        if !items.is_empty() {
            menus.push(gpui::Menu {
                name: menu.into(),
                items,
            });
        }
    }
    menus
}
