//! Editor actions come from one registry (`kerf-commands`): each command's
//! id, label, menu and shortcuts in the chosen keymap. The window binds the
//! shortcuts to [`RunCommand`] and carries the command out by its id;
//! engine commands it does not carry out itself resolve to the bridge
//! request that does.

use gpui::{App, KeyBinding, NoAction, SharedString};
use serde_json::Value;

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

fn chord(chord: &str) -> Option<String> {
    let chord = chord.trim();
    if chord.is_empty() {
        return None;
    }
    // "Ctrl++" names the plus key.
    let (modifiers, key) = match chord.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None => match chord.rfind('+') {
            Some(at) => (&chord[..at], &chord[at + 1..]),
            None => ("", chord),
        },
    };
    let mut parts = Vec::new();
    for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
        parts.push(match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => "secondary",
            "meta" => {
                if cfg!(target_os = "macos") {
                    "ctrl"
                } else {
                    "super"
                }
            }
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
    // Esc puts away what is open over the window before the window's own Esc
    // (Reject Proposal) has it: their contexts are the deeper ones.
    crate::settings::bind_close(cx);
    crate::export::bind_close(cx);
    cx.set_menus(menus(keymap));
}

/// The registry's menus, for a menu bar (macOS draws it; elsewhere the
/// command palette and the shortcuts reach the same commands).
pub fn menus(keymap: &str) -> Vec<gpui::Menu> {
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
            items.push(gpui::MenuItem::action(
                command.label.clone(),
                RunCommand {
                    id: command.id.clone().into(),
                },
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
