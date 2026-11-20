//! Editor actions come from one registry (`kerf-commands`): each command's
//! id, label, menu and shortcuts in the chosen keymap. The window binds the
//! shortcuts to [`RunCommand`] and carries the command out by its id;
//! engine commands it does not carry out itself resolve to the bridge
//! request that does.

use gpui::{App, KeyBinding, NoAction, SharedString};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Run the registry's command `id`.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct RunCommand {
    pub id: SharedString,
}

impl RunCommand {
    pub fn new(id: &'static str) -> Self {
        Self { id: id.into() }
    }
}

/// The key context of the editor's window.
pub const WORKSPACE: &str = "Workspace";

/// The key context of the timeline, where its own commands' keys work.
pub const TIMELINE: &str = "Timeline";

// Escape: Esc outside a field stops a preview rendering, else puts the Media
// band away (bound in the window, below whatever is open over it).
// RevealPicture: something in the conversation asks to be looked at (a
// timecode, a proposal's Preview): the picture comes in if it is away.
gpui::actions!(kerf, [Escape, RevealPicture]);

/// File ▸ Open Recent: open the project at `path` in place of this window
/// (the window's own kerfd kept). The sidebar's other folders open the same way.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct OpenRecent {
    pub path: SharedString,
}

// ClearRecent: File ▸ Open Recent ▸ Clear Menu forgets the recent projects.
// RecentMissing and NoRecent: rows of Open Recent that cannot be chosen (a
// folder that is gone; "No Recent Projects"). Nothing carries them out, so
// a menu bar shows them greyed.
gpui::actions!(kerf, [ClearRecent, RecentMissing, NoRecent]);

// Unavailable: a menu bar row of the registry's that cannot run now (see
// [`MenuStates`]). Nothing carries it out, so the menu bar greys it.
gpui::actions!(kerf, [Unavailable]);

/// How the registry's commands stand in the window in front, for the menu
/// bar: those that cannot run now (greyed), and the switches that are on
/// (ticked). The window works them out as it draws ([`set_menu_states`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuStates {
    pub unavailable: BTreeSet<String>,
    pub checked: BTreeSet<String>,
}

/// The menu bar's states as last set, and the project then in front.
#[derive(Default)]
struct ShownStates {
    states: MenuStates,
    current: Option<PathBuf>,
}

impl gpui::Global for ShownStates {}

/// Sets the menu bar again when how its commands stand (`states`) or the
/// project in front (`current`) has changed since it was last set. The
/// window in front calls it as it draws.
pub fn set_menu_states(states: MenuStates, current: Option<&Path>, cx: &mut App) {
    let shown = cx.default_global::<ShownStates>();
    if shown.states == states && shown.current.as_deref() == current {
        return;
    }
    shown.states = states;
    shown.current = current.map(Path::to_path_buf);
    refresh_menus(current, cx);
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

/// One row of File ▸ Open Recent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentRow {
    /// The folder's name.
    pub name: String,
    pub path: PathBuf,
    /// The project this window has open: checked.
    pub current: bool,
    /// Its folder is gone: shown, and not chosen.
    pub missing: bool,
}

/// File ▸ Open Recent's rows, newest first: each recent project by its
/// folder's name, the one open in this window checked, one whose folder is
/// gone greyed. Empty with none.
pub fn recent_rows(recent: &[PathBuf], current: Option<&Path>) -> Vec<RecentRow> {
    recent
        .iter()
        .map(|path| RecentRow {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            path: path.clone(),
            current: current.is_some_and(|current| crate::app::same_folder(current, path)),
            missing: !path.is_dir(),
        })
        .collect()
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

/// Sets the menu bar again, for the keymap and recent projects as the
/// editor's settings have them now and with `current` (the project of the
/// window in front) checked in Open Recent. A window calls it as it comes
/// to the front, and after Open Recent ▸ Clear Menu.
pub fn refresh_menus(current: Option<&Path>, cx: &mut App) {
    let prefs = crate::prefs::Prefs::load();
    let states = cx.default_global::<ShownStates>().states.clone();
    cx.set_menus(menus_with_states(
        prefs.keymap.wire(),
        &prefs.recent,
        current,
        &states,
    ));
}

/// A registry shortcut ("Ctrl+Shift+E") as the system writes it: on macOS
/// in its signs ("⇧⌘E"), elsewhere as written, but with Meta+ written
/// Ctrl+, as [`keystroke`] binds it there (Qt's menus said so too). Each
/// chord of a sequence in turn, a comma between.
pub fn shown_keys(shortcut: &str) -> String {
    if !cfg!(target_os = "macos") {
        return shortcut
            .split(", ")
            .map(|chord| {
                let (modifiers, key) = split_chord(chord.trim());
                let mut shown: Vec<&str> = Vec::new();
                for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
                    let modifier = if modifier.eq_ignore_ascii_case("meta") {
                        "Ctrl"
                    } else {
                        modifier
                    };
                    if !shown
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(modifier))
                    {
                        shown.push(modifier);
                    }
                }
                shown.push(key);
                shown.join("+")
            })
            .collect::<Vec<_>>()
            .join(", ");
    }
    shortcut
        .split(", ")
        .map(|chord| {
            let (modifiers, key) = split_chord(chord.trim());
            let has = |names: &[&str]| {
                modifiers
                    .split('+')
                    .any(|part| names.iter().any(|name| part.eq_ignore_ascii_case(name)))
            };
            let mut shown = String::new();
            // The order macOS writes them in: ⌃⌥⇧⌘.
            if has(&["meta"]) {
                shown.push('⌃');
            }
            if has(&["alt", "option"]) {
                shown.push('⌥');
            }
            if has(&["shift"]) {
                shown.push('⇧');
            }
            if has(&["ctrl", "cmd", "command"]) {
                shown.push('⌘');
            }
            shown.push_str(match key.to_ascii_lowercase().as_str() {
                "return" | "enter" => "↩",
                "esc" | "escape" => "⎋",
                "backspace" => "⌫",
                "del" | "delete" => "⌦",
                "left" => "←",
                "right" => "→",
                "up" => "↑",
                "down" => "↓",
                "space" => "Space",
                "tab" => "⇥",
                _ => key,
            });
            shown
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One command as the catalog lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub menu: String,
    /// Its group within the menu: a separator falls between groups.
    pub group: i64,
    /// `window`, or `timeline` for a command whose keys work only while the
    /// timeline has them, so typing or Esc elsewhere never runs it.
    pub scope: String,
    pub keys: Vec<String>,
    /// A switch or one of a choice (Safe Zones, Follow Kerf, a tool): its
    /// menu row and its palette row are ticked while it is on.
    pub checkable: bool,
}

/// The registry in `keymap` (`kerf`, `premiere`, `fcp`).
pub fn catalog(keymap: &str) -> Vec<Command> {
    let catalog = kerf_bridge::commands_catalog(keymap);
    catalog["commands"]
        .as_array()
        .map(|commands| {
            commands
                .iter()
                .map(|command| Command {
                    id: command["id"].as_str().unwrap_or_default().to_owned(),
                    label: command["label"].as_str().unwrap_or_default().to_owned(),
                    menu: command["menu"].as_str().unwrap_or_default().to_owned(),
                    group: command["group"].as_i64().unwrap_or(0),
                    scope: command["scope"].as_str().unwrap_or("window").to_owned(),
                    checkable: command["checkable"].as_bool().unwrap_or(false),
                    keys: command["keys"]
                        .as_array()
                        .map(|keys| {
                            keys.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A registry shortcut ("Ctrl+Shift+Z", "Space", "Ctrl+K, Ctrl+S") as GPUI
/// writes keystrokes ("secondary-shift-z"). Ctrl is the platform's command
/// key, as the registry means it; Meta is the other one. None for a key
/// GPUI has no name for.
pub fn keystroke(shortcut: &str) -> Option<String> {
    let chords: Option<Vec<String>> = shortcut.split(", ").map(chord).collect();
    chords.map(|chords| chords.join(" "))
}

/// A chord's modifiers ("Ctrl+Shift") and its key ("Z"). "Ctrl++" and a
/// lone "+" name the plus key.
fn split_chord(chord: &str) -> (&str, &str) {
    match chord.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None => match chord.rfind('+') {
            Some(at) if at > 0 => (&chord[..at], &chord[at + 1..]),
            _ => ("", chord),
        },
    }
}

fn chord(chord: &str) -> Option<String> {
    let chord = chord.trim();
    if chord.is_empty() {
        return None;
    }
    let (modifiers, key) = split_chord(chord);
    let mut parts = Vec::new();
    for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
        parts.push(match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => "secondary",
            // Control on macOS. Elsewhere the system keeps the Super key's
            // shortcuts for itself, so Meta+ is bound as Ctrl+ there.
            "meta" => "ctrl",
            "alt" | "option" => "alt",
            "shift" => "shift",
            _ => return None,
        });
    }
    let key = match key.to_ascii_lowercase().as_str() {
        "space" => "space".to_owned(),
        "return" | "enter" => "enter".to_owned(),
        "esc" | "escape" => "escape".to_owned(),
        "del" | "delete" => "delete".to_owned(),
        "backspace" => "backspace".to_owned(),
        "tab" => "tab".to_owned(),
        "left" | "right" | "up" | "down" | "home" | "end" => key.to_ascii_lowercase(),
        "pgup" | "pageup" => "pageup".to_owned(),
        "pgdown" | "pagedown" => "pagedown".to_owned(),
        other if other.chars().count() == 1 => other.to_owned(),
        other if other.starts_with('f') && other[1..].parse::<u8>().is_ok() => other.to_owned(),
        _ => return None,
    };
    parts.push(&key);
    Some(parts.join("-"))
}

/// Whether a keystroke has no modifier but Shift: typing it in a field
/// types it, so it is unbound there.
fn plain(keystroke: &str) -> bool {
    let first = keystroke.split(' ').next().unwrap_or(keystroke);
    !["secondary-", "ctrl-", "alt-", "super-", "cmd-"]
        .iter()
        .any(|modifier| first.contains(modifier))
}

/// Binds every command's shortcuts in `keymap`.
pub fn bind(keymap: &str, cx: &mut App) {
    cx.clear_key_bindings();
    crate::input::bind_keys(cx);
    let mut bindings = Vec::new();
    for command in catalog(keymap) {
        let id: SharedString = command.id.clone().into();
        for shortcut in &command.keys {
            let Some(keystroke) = keystroke(shortcut) else {
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
                    Some(crate::input::CONTEXT),
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
        crate::palette::OpenPalette,
        Some(WORKSPACE),
    ));
    bindings.push(KeyBinding::new("escape", Escape, Some(WORKSPACE)));
    bindings.push(KeyBinding::new(
        "up",
        crate::palette::Previous,
        Some(crate::palette::CONTEXT),
    ));
    bindings.push(KeyBinding::new(
        "down",
        crate::palette::Next,
        Some(crate::palette::CONTEXT),
    ));
    cx.bind_keys(bindings);
    // What the views bind of their own, which binding a keymap cleared: the
    // picture's mask tools and its Delete, Esc and Return; the sidebar's
    // Tab, menus and rows.
    crate::stage::bind_keys(cx);
    crate::sidebar::bind_keys(cx);
    // Esc puts away what is open over the window before the window's own Esc
    // (Reject Proposal) has it: their contexts are the deeper ones.
    crate::settings::bind_close(cx);
    crate::export::bind_close(cx);
    // A shot's graph's Esc and its canvas's keys, should a keymap be bound
    // while one is open.
    crate::making::bind_keys(cx);
    let recent = crate::prefs::Prefs::load().recent;
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
    let mut menus = vec![gpui::Menu {
        name: "Industrial Kerf".into(),
        items: vec![
            gpui::MenuItem::action("About Kerf", RunCommand::new("help.about")),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Settings…", RunCommand::new("app.preferences")),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Quit Industrial Kerf", RunCommand::new("app.quit")),
        ],
    }];
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
        // The picture's mask tools, which the registry does not carry, where
        // Qt's View menu had them (their keys, ⇧R ⇧E ⇧P, the stage binds).
        if menu == "View" && !items.is_empty() {
            items.push(gpui::MenuItem::separator());
            items.push(gpui::MenuItem::action(
                crate::stage::MaskTool::Rectangle.label(),
                crate::stage::RectangleMask,
            ));
            items.push(gpui::MenuItem::action(
                crate::stage::MaskTool::Ellipse.label(),
                crate::stage::EllipseMask,
            ));
            items.push(gpui::MenuItem::action(
                crate::stage::MaskTool::Pen.label(),
                crate::stage::PenMask,
            ));
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
