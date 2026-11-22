//! The app's menu, opened from the account at the sidebar's foot (or the
//! toolbar's ⋯ while the sidebar is away), built as rows of data first:
//! Settings…, Account…; Media and Import…; View ▸ (the panes, the mask
//! tools, the zoom, the keyboard, the sync); the registry's menus where
//! there is no menu bar; Sign Out while signed in. And the actions its rows
//! dispatch that the window carries out.

use editor_session::commands::{OpenRecent, RunCommand};
use editor_state::prefs::Keymap;
use editor_ui::icons::Icon;
use editor_ui::ui;
use gpui::SharedString;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Choose the keymap the editor answers to (the app's menu, View ▸ Keyboard).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct ChooseKeymap {
    pub keymap: Keymap,
}

/// Switch to another branch of the project's repository (View ▸ Sync).
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = kerf, no_json)]
pub struct SwitchBranch {
    pub name: SharedString,
}

// SignOut: out of the Kerf account (the app's menu, while signed in).
gpui::actions!(kerf, [SignOut]);

/// What a row of the app's menu does, chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum AppDoes {
    /// Nothing: words that say how something stands.
    Nothing,
    /// The registry's command (or one the window carries out) by its id.
    Command(String),
    SignOut,
    /// A mask tool on the picture.
    Mask(editor_stage::stage::MaskTool),
    /// The viewer's zoom, a percent of the frame's own pixels; 0 fits it.
    Zoom(u32),
    Keymap(Keymap),
    Branch(String),
    /// A recent project, in place of this one.
    Recent(PathBuf),
    ClearRecent,
}

impl AppDoes {
    /// The action it dispatches from where the keys are (the window).
    pub fn action(&self) -> Option<Box<dyn gpui::Action>> {
        Some(match self {
            AppDoes::Nothing => return None,
            AppDoes::Command(id) => Box::new(RunCommand {
                id: id.clone().into(),
            }),
            AppDoes::SignOut => Box::new(SignOut),
            AppDoes::Mask(tool) => match tool {
                editor_stage::stage::MaskTool::Rectangle => {
                    Box::new(editor_stage::stage::RectangleMask)
                }
                editor_stage::stage::MaskTool::Ellipse => {
                    Box::new(editor_stage::stage::EllipseMask)
                }
                editor_stage::stage::MaskTool::Pen => Box::new(editor_stage::stage::PenMask),
            },
            AppDoes::Zoom(percent) => Box::new(editor_stage::stage::SetZoom { percent: *percent }),
            AppDoes::Keymap(keymap) => Box::new(ChooseKeymap { keymap: *keymap }),
            AppDoes::Branch(name) => Box::new(SwitchBranch {
                name: name.clone().into(),
            }),
            AppDoes::Recent(path) => Box::new(OpenRecent {
                path: path.to_string_lossy().into_owned().into(),
            }),
            AppDoes::ClearRecent => Box::new(editor_session::commands::ClearRecent),
        })
    }
}

/// A row of the app's menu, as it shows: its words, icon and key, whether
/// it is ticked or greyed (with why in its tip), and the rows it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRow {
    pub words: String,
    pub icon: Option<Icon>,
    pub key: Option<String>,
    pub checked: bool,
    pub disabled: bool,
    pub tip: Option<String>,
    /// A line goes before it.
    pub separated: bool,
    pub does: AppDoes,
    /// A submenu's rows.
    pub rows: Vec<AppRow>,
}

impl AppRow {
    fn new(words: impl Into<String>, does: AppDoes) -> Self {
        Self {
            words: words.into(),
            icon: None,
            key: None,
            checked: false,
            disabled: false,
            tip: None,
            separated: false,
            does,
            rows: Vec::new(),
        }
    }

    fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    fn separated(mut self) -> Self {
        self.separated = true;
        self
    }

    fn submenu(words: impl Into<String>, rows: Vec<AppRow>) -> Self {
        Self {
            rows,
            ..Self::new(words, AppDoes::Nothing)
        }
    }

    /// Words that say how something stands: greyed, chosen by nothing.
    fn note(words: impl Into<String>) -> Self {
        Self {
            disabled: true,
            ..Self::new(words, AppDoes::Nothing)
        }
    }
}

/// What the app's menu is built from: how the window and its commands
/// stand ([`app_rows`]).
pub struct AppMenuInput<'a> {
    /// Signed in to Kerf: Sign Out is offered.
    pub signed_in: bool,
    pub keymap: Keymap,
    /// The registry's commands in the keymap in use ([`editor_session::commands::catalog`]),
    /// and its menus in order.
    pub commands: &'a [editor_session::commands::Command],
    pub menus: &'a [String],
    /// Why each command cannot run now (`kerf_bridge::commands_states`).
    pub states: &'a Value,
    /// The checkable commands that are on.
    pub checked: &'a std::collections::BTreeSet<String>,
    /// The viewer's zoom (0 fits).
    pub zoom: f32,
    /// The project's Git standing (the last `collab` event's `status`),
    /// with why the last sync failed under [`SYNC_FAILURE`] when it did.
    pub collab: Option<&'a Value>,
    pub revision: u64,
    /// The system draws the registry's menus in a menu bar (macOS):
    /// they are not repeated here.
    pub menu_bar: bool,
    pub recent: &'a [PathBuf],
    pub current: Option<&'a Path>,
}

/// The row for the registry's command `id`, in `words` (its own label
/// when None): its key, ticked while on, greyed with why while it cannot
/// run.
fn command_row(input: &AppMenuInput, id: &str, words: Option<&str>) -> AppRow {
    let command = input.commands.iter().find(|command| command.id == id);
    let reason = editor_session::palette::unavailable(input.states, id).map(str::to_owned);
    AppRow {
        key: command
            .and_then(|command| command.keys.first())
            .map(|key| editor_state::shortcuts::shown_keys(key)),
        checked: command.is_some_and(|command| command.checkable) && input.checked.contains(id),
        disabled: reason.is_some(),
        tip: reason,
        ..AppRow::new(
            words
                .map(str::to_owned)
                .or_else(|| command.map(|command| command.label.clone()))
                .unwrap_or_else(|| id.to_owned()),
            AppDoes::Command(id.to_owned()),
        )
    }
}

/// The row for the picture's mask tool `tool`, with its key: ticked while
/// it is out, greyed with why while none can be (no picture clip chosen
/// that the picture can place), as the states and ticks say by its own id
/// ([`editor_stage::stage::mask_tool_id`]).
fn mask_row(input: &AppMenuInput, tool: editor_stage::stage::MaskTool) -> AppRow {
    let id = editor_stage::stage::mask_tool_id(tool);
    let reason = editor_session::palette::unavailable(input.states, id).map(str::to_owned);
    AppRow {
        key: Some(editor_state::shortcuts::shown_keys(tool.shortcut())),
        checked: input.checked.contains(id),
        disabled: reason.is_some(),
        tip: reason,
        ..AppRow::new(tool.label(), AppDoes::Mask(tool))
    }
}

/// The app's menu: Settings…, Account…; Media and Import…; View ▸ (the
/// panes, Follow Kerf, the mask tools, Safe Zones, Viewer Zoom ▸,
/// Keyboard ▸, Sync ▸); where there is no menu bar, the registry's menus
/// (File ▸ … Help ▸); Sign Out while signed in to Kerf.
pub fn app_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let mut rows = vec![
        command_row(input, "app.preferences", Some("Settings…")).icon(Icon::Gear),
        AppRow::new("Account…", AppDoes::Command("app.account".into())).icon(Icon::Lock),
        command_row(input, "window.project", Some("Media"))
            .icon(Icon::Media)
            .separated(),
        command_row(input, "file.import", Some("Import…")).icon(Icon::Import),
        AppRow::submenu("View", view_rows(input)).icon(Icon::Eye),
    ];
    if !input.menu_bar {
        let mut first = true;
        for menu in input.menus {
            let menu_rows = registry_rows(input, menu);
            if menu_rows.is_empty() {
                continue;
            }
            let mut row = AppRow::submenu(menu.clone(), menu_rows);
            row.separated = std::mem::take(&mut first);
            rows.push(row);
        }
    }
    if input.signed_in {
        rows.push(AppRow::new("Sign Out", AppDoes::SignOut).separated());
    }
    rows
}

/// View ▸: the panes and Follow Kerf, the mask tools, Safe Zones and the
/// zoom, the keyboard's choice and the sync.
fn view_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let mut rows = vec![
        command_row(input, "view.sidebar", Some("Sidebar")),
        command_row(input, "view.picture", Some("Picture")),
        command_row(input, "view.timeline", Some("Timeline")),
        command_row(input, "view.followKerf", Some("Follow Kerf")),
    ];
    for (index, tool) in editor_stage::stage::MaskTool::ALL.into_iter().enumerate() {
        let mut row = mask_row(input, tool).icon(tool.icon());
        row.separated = index == 0;
        rows.push(row);
    }
    rows.push(command_row(input, "view.safeZones", Some("Safe Zones")).separated());
    let zooms = editor_stage::stage::ZOOMS
        .into_iter()
        .map(|zoom| AppRow {
            checked: (zoom - input.zoom).abs() < 1e-3,
            ..AppRow::new(
                editor_stage::stage::zoom_words(zoom),
                AppDoes::Zoom((zoom * 100.0).round() as u32),
            )
        })
        .collect();
    rows.push(AppRow::submenu("Viewer Zoom", zooms));
    let keymaps = Keymap::ALL
        .into_iter()
        .map(|keymap| AppRow {
            checked: keymap == input.keymap,
            ..AppRow::new(keymap.label(), AppDoes::Keymap(keymap))
        })
        .collect();
    rows.push(AppRow::submenu("Keyboard", keymaps).separated());
    rows.push(AppRow::submenu("Sync", sync_rows(input)));
    rows
}

/// What a sync's standing says over its rows: the revision, then how far
/// it is from its upstream ("Revision 22 · 1 commit to pull").
pub fn sync_summary(collab: Option<&Value>, revision: u64) -> String {
    let revision = format!("Revision {revision}");
    let Some(status) = collab.filter(|status| status["repository"].as_bool() == Some(true)) else {
        return format!("{revision} · not shared yet");
    };
    let count = |name: &str| status[name].as_i64().unwrap_or(0).max(0);
    let (behind, ahead) = (count("behind"), count("ahead"));
    let mut parts = vec![revision];
    let plural = |n: i64, one: &str, many: &str| {
        if n == 1 {
            one.to_owned()
        } else {
            many.replace("{}", &n.to_string())
        }
    };
    if behind > 0 {
        parts.push(plural(behind, "1 commit to pull", "{} commits to pull"));
    }
    if ahead > 0 {
        parts.push(plural(ahead, "1 commit to push", "{} commits to push"));
    }
    if behind == 0 && ahead == 0 {
        parts.push("up to date".into());
    }
    parts.join(" · ")
}

/// Where the window puts why the last sync failed (until one succeeds)
/// in the standing it gives the app's menu ([`AppMenuInput::collab`]):
/// the Sync submenu says so ([`with_sync_failure`]).
pub const SYNC_FAILURE: &str = "syncFailure";

/// `collab` (the project's Git standing) with why the last sync failed,
/// when it did, for the app's menu ([`SYNC_FAILURE`]).
pub fn with_sync_failure(collab: Option<&Value>, failure: Option<&str>) -> Option<Value> {
    let mut status = collab.cloned()?;
    if let (Some(failure), Some(fields)) = (failure, status.as_object_mut()) {
        fields.insert(SYNC_FAILURE.to_owned(), Value::from(failure));
    }
    Some(status)
}

/// What the Sync submenu warns of under its summary: "A merge is
/// unfinished; finish it with git" while one is, else "Sync failed: <why>"
/// after a sync that failed; none when nothing went wrong.
pub fn sync_problem(collab: Option<&Value>) -> Option<String> {
    let status = collab?;
    if status["conflicted"].as_bool() == Some(true) {
        return Some("A merge is unfinished; finish it with git".to_owned());
    }
    status[SYNC_FAILURE]
        .as_str()
        .filter(|failure| !failure.trim().is_empty())
        .map(|failure| format!("Sync failed: {failure}"))
}

/// View ▸ Sync: how it stands; the branches (this one ticked; another
/// switches to it) and New Branch…; Sync Now, Live Sync, Commit… and Pull
/// Request…. A project that is no repository yet offers Publish only.
fn sync_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let mut rows = vec![AppRow::note(sync_summary(input.collab, input.revision))];
    let status = input
        .collab
        .filter(|status| status["repository"].as_bool() == Some(true));
    // What went wrong, when something did: an unfinished merge, else why
    // the last sync failed.
    if let Some(problem) = sync_problem(input.collab) {
        rows.push(AppRow {
            icon: Some(Icon::Dot),
            // All of it, where the row cuts it to one line.
            tip: Some(problem.clone()),
            ..AppRow::note(problem)
        });
    }
    let Some(status) = status else {
        rows.push(command_row(input, "collab.publish", None).separated());
        return rows;
    };
    let current = status["branch"].as_str().unwrap_or_default().to_owned();
    let mut branches: Vec<String> = Vec::new();
    if !current.is_empty() {
        branches.push(current.clone());
    }
    for branch in status["branches"].as_array().into_iter().flatten() {
        if let Some(branch) = branch.as_str().filter(|branch| !branch.is_empty())
            && !branches.iter().any(|known| known == branch)
        {
            branches.push(branch.to_owned());
        }
    }
    let switch_reason = editor_session::palette::unavailable(input.states, "collab.switchBranch")
        .map(str::to_owned);
    for (index, branch) in branches.into_iter().enumerate() {
        let here = branch == current;
        rows.push(AppRow {
            checked: here,
            disabled: !here && switch_reason.is_some(),
            tip: (!here).then(|| switch_reason.clone()).flatten(),
            separated: index == 0,
            ..AppRow::new(
                branch.clone(),
                if here {
                    AppDoes::Nothing
                } else {
                    AppDoes::Branch(branch)
                },
            )
        });
    }
    rows.push(command_row(input, "collab.newBranch", None));
    rows.push(command_row(input, "collab.sync", None).separated());
    rows.push(command_row(input, "collab.live", None));
    rows.push(command_row(input, "collab.commit", None));
    rows.push(command_row(input, "collab.pullRequest", None));
    rows
}

/// The registry's menu `menu` as rows, a line between its groups; File ▸
/// Open Recent after Open Project, and the mask tools at View's end, as the
/// menu bar has them.
fn registry_rows(input: &AppMenuInput, menu: &str) -> Vec<AppRow> {
    let mut rows = Vec::new();
    let mut group = None;
    for command in input.commands.iter().filter(|command| command.menu == menu) {
        let mut row = command_row(input, &command.id, None);
        // Switch Branch needs a branch: it opens on the others, greyed
        // with why while there are none to go to.
        if command.id == "collab.switchBranch" {
            row = AppRow {
                rows: other_branches(input),
                does: AppDoes::Nothing,
                ..row
            };
        }
        row.separated = group.is_some_and(|group| group != command.group);
        group = Some(command.group);
        rows.push(row);
        if command.id == "file.openProject" {
            rows.push(AppRow::submenu("Open Recent", recent_rows(input)));
        }
    }
    if menu == "View" && !rows.is_empty() {
        for (index, tool) in editor_stage::stage::MaskTool::ALL.into_iter().enumerate() {
            let mut row = mask_row(input, tool);
            row.separated = index == 0;
            rows.push(row);
        }
    }
    rows
}

/// Collaborate ▸ Switch Branch: the project's branches other than the one
/// it is on, each switching to it; "No Other Branches" when there are
/// none.
fn other_branches(input: &AppMenuInput) -> Vec<AppRow> {
    let current = input
        .collab
        .and_then(|status| status["branch"].as_str())
        .unwrap_or_default();
    let mut rows: Vec<AppRow> = Vec::new();
    for branch in input
        .collab
        .and_then(|status| status["branches"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|branch| !branch.is_empty() && *branch != current)
    {
        if !rows.iter().any(|row| row.words == branch) {
            rows.push(AppRow::new(branch, AppDoes::Branch(branch.to_owned())));
        }
    }
    if rows.is_empty() {
        rows.push(AppRow::note(editor_finish::finish::NO_OTHER_BRANCHES));
    }
    rows
}

/// File ▸ Open Recent: each recent project (the one open here ticked, one
/// whose folder is gone greyed), "No Recent Projects" when there are none,
/// then Clear Menu.
fn recent_rows(input: &AppMenuInput) -> Vec<AppRow> {
    let recent = editor_session::commands::recent_rows(input.recent, input.current);
    let mut rows: Vec<AppRow> = recent
        .iter()
        .map(|row| AppRow {
            checked: row.current,
            disabled: row.missing,
            tip: Some(if row.missing {
                editor_session::palette::RECENT_GONE.to_owned()
            } else {
                row.path.display().to_string()
            }),
            ..AppRow::new(row.name.clone(), AppDoes::Recent(row.path.clone()))
        })
        .collect();
    if rows.is_empty() {
        rows.push(AppRow::note("No Recent Projects"));
    }
    rows.push(AppRow {
        disabled: recent.is_empty(),
        ..AppRow::new("Clear Menu", AppDoes::ClearRecent).separated()
    });
    rows
}

/// `rows` as the kit's menu rows: each, chosen, dispatches what it does
/// from where the keys go back to once the menu has gone.
pub fn app_menu_items(rows: &[AppRow]) -> Vec<ui::MenuItem> {
    rows.iter()
        .map(|row| {
            let mut item = ui::MenuItem::new(row.words.clone())
                .checked(row.checked)
                .disabled(row.disabled);
            if let Some(icon) = row.icon {
                item = item.icon(icon);
            }
            if let Some(key) = &row.key {
                item = item.key(key.clone());
            }
            if let Some(tip) = &row.tip {
                item = item.tip(tip.clone());
            }
            if row.separated {
                item = item.separated();
            }
            if !row.rows.is_empty() {
                item = item.submenu(app_menu_items(&row.rows));
            }
            let does = row.does.clone();
            item.on_choose(move |window, cx| {
                if let Some(action) = does.action() {
                    window.dispatch_action(action, cx);
                }
            })
        })
        .collect()
}
