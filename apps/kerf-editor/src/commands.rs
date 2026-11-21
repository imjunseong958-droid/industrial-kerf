//! Editor actions come from one registry (`kerf-commands`): each command's
//! id, label, menu and shortcuts in the chosen keymap. The window binds the
//! shortcuts to [`RunCommand`] and carries the command out by its id;
//! engine commands it does not carry out itself resolve to the bridge
//! request that does.

use gpui::{App, SharedString};
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
// NoOtherBranches: Collaborate ▸ Switch Branch's row when there is no
// branch but this one; greyed the same way.
gpui::actions!(kerf, [Unavailable, NoOtherBranches]);

// The macOS application menu's own rows, as Qt's Cocoa menu had them:
// HideApp hides Industrial Kerf (⌘H), HideOtherApps the other applications
// (⌥⌘H), ShowAllApps shows them again ([`keymap::answer_app_menu`]).
gpui::actions!(kerf, [HideApp, HideOtherApps, ShowAllApps]);

/// Why no mask tool can be taken out now, as the picture says it.
pub const NO_MASK_CLIP: &str = "Choose a picture clip to draw a mask on";

/// Collaborate ▸ Switch Branch in the menu bar: the branches of the
/// window in front other than the one it is on, and whether switching is
/// held back now (as a sync is: kerfd away, project.kerf changed on disk,
/// no repository). The window's sync chip sets it as it draws
/// ([`set_menu_branches`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuBranches {
    pub others: Vec<String>,
    pub blocked: bool,
}

impl gpui::Global for MenuBranches {}

/// Sets the menu bar again when the branches of the window in front have
/// changed since they were last set.
pub fn set_menu_branches(branches: MenuBranches, cx: &mut App) {
    if cx.try_global::<MenuBranches>() == Some(&branches) {
        return;
    }
    cx.set_global(branches);
    let current = cx.default_global::<ShownStates>().current.clone();
    refresh_menus(current.as_deref(), cx);
}

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
            current: current.is_some_and(|current| crate::folders::same_folder(current, path)),
            missing: !path.is_dir(),
        })
        .collect()
}

/// What only the assembly (`keymap`) can do for the views, which cannot
/// name it: bind a keymap (each view's own keys with it) and build the menu
/// bar. `keymap::bind` sets it as it first binds, before any window opens.
#[derive(Clone, Copy)]
pub struct Assembly {
    pub bind: fn(&str, &mut App),
    pub menus: BuildMenus,
}

impl gpui::Global for Assembly {}

/// How the assembly builds the menu bar: for a keymap, with the recent
/// projects (the current one checked), how the commands stand, and the
/// branches to switch to.
pub type BuildMenus =
    fn(&str, &[PathBuf], Option<&Path>, &MenuStates, &MenuBranches) -> Vec<gpui::Menu>;

/// Binds `keymap` as the assembly does: Settings' keymap chooser calls it.
pub fn rebind(keymap: &str, cx: &mut App) {
    if let Some(assembly) = cx.try_global::<Assembly>().copied() {
        (assembly.bind)(keymap, cx);
    }
}

/// Sets the menu bar again, for the keymap and recent projects as the
/// editor's settings have them now and with `current` (the project of the
/// window in front) checked in Open Recent. A window calls it as it comes
/// to the front, and after Open Recent ▸ Clear Menu. The menus are the
/// [`Assembly`]'s.
pub fn refresh_menus(current: Option<&Path>, cx: &mut App) {
    let prefs = crate::prefs::Prefs::load();
    let states = cx.default_global::<ShownStates>().states.clone();
    let branches = cx.default_global::<MenuBranches>().clone();
    let Some(assembly) = cx.try_global::<Assembly>().copied() else {
        return;
    };
    cx.set_menus((assembly.menus)(
        prefs.keymap.wire(),
        &prefs.recent,
        current,
        &states,
        &branches,
    ));
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
