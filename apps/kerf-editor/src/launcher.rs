//! The project launcher: what opens with no project. Kerf's mark and one
//! question, the ways to begin (a new project, one from a shared preset, a
//! folder, a clone from GitHub), and the projects opened before. Making and
//! cloning run as the bridge's project tasks, on a thread of their own,
//! polled until they end. Settings opens over it as over a project's
//! window, with no project: what answers in the conversation, its key, how
//! the editor looks, its keys.
//!
//! It works from the keyboard as well: Tab and Shift-Tab go from control to
//! control, Up and Down choose among the recent projects, Return does what
//! the page is for (Create, Clone, or opening the project chosen), Esc
//! puts the form away.

use crate::commands::{RunCommand, WORKSPACE};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::motion;
use crate::prefs::{self, Prefs};
use crate::presets::{self, Fit, Picked, PresetChoice};
use crate::session::Session;
use crate::settings::{Page, SettingsEvent, SettingsView};
use crate::theme::{self, height, radius, text};
use crate::ui::{self, icon_button};
use gpui::{
    div, img, prelude::*, px, AnimationExt, App, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, KeyBinding, Keystroke, MouseButton, PathPromptOptions,
    PromptButton, PromptLevel, SharedString, Subscription, Task, Window,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

pub enum LauncherEvent {
    /// Open `project`; `starts` when the folder was no project yet, so its
    /// media comes in with it. What making or cloning it warned of is said
    /// in its window once it is open ([`crate::app::warn_when_open`]).
    Open { project: PathBuf, starts: bool },
}

gpui::actions!(
    launcher,
    [
        NextControl,
        PreviousControl,
        Press,
        Choose,
        Back,
        ChosenUp,
        ChosenDown,
        Earlier,
        Later
    ]
);

/// The launcher's key context: Tab, the arrows, Return and Esc.
const CONTEXT: &str = "Launcher";
/// A control's own key context, where Space and Return press it.
const CONTROL: &str = "LauncherControl";

/// How often a running project task is asked how it is getting on.
const TASK_POLL: Duration = Duration::from_millis(100);
/// The launcher's column, and the air above it.
const COLUMN: f32 = 640.0;
const TOP: f32 = 88.0;
/// A form's labels stand in a column this wide.
const FORM_LABEL: f32 = 96.0;
/// A recent project's row, and a preset's in the From Preset page.
const RECENT_ROW: f32 = 52.0;
const PRESET_ROW: f32 = 44.0;
/// A slot's start field.
const START_FIELD: f32 = 64.0;
/// The ring round the control the keyboard is on.
const RING: f32 = 2.0;

/// What is being begun, below the ways to begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    New,
    Preset,
    Clone,
}

impl Mode {
    /// As the launcher's way is kept in the editor's settings.
    pub fn wire(self) -> &'static str {
        match self {
            Mode::New => "new",
            Mode::Preset => "preset",
            Mode::Clone => "clone",
        }
    }

    pub fn from_wire(wire: &str) -> Option<Mode> {
        [Mode::New, Mode::Preset, Mode::Clone]
            .into_iter()
            .find(|mode| mode.wire() == wire)
    }
}

/// A new project's frame: kerf-collab's templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Standard,
    Vertical,
    Square,
}

impl Format {
    const ALL: [Format; 3] = [Format::Standard, Format::Vertical, Format::Square];

    fn label(self) -> &'static str {
        match self {
            Format::Standard => "16:9",
            Format::Vertical => "9:16",
            Format::Square => "1:1",
        }
    }

    fn size(self) -> &'static str {
        match self {
            Format::Standard => "1920 × 1080",
            Format::Vertical => "1080 × 1920",
            Format::Square => "1080 × 1080",
        }
    }

    fn wire(self) -> &'static str {
        match self {
            Format::Standard => "standard",
            Format::Vertical => "vertical",
            Format::Square => "square",
        }
    }
}

/// The frame rates a new project can have, as documents write them.
const RATES: [(&str, &str); 6] = [
    ("23.976", "23.976"),
    ("24", "24"),
    ("25", "25"),
    ("29.97", "29.97"),
    ("30", "30"),
    ("60", "60"),
];

/// A project opened before.
#[derive(Clone, Debug)]
struct Recent {
    path: PathBuf,
    /// Its document's name, else its folder's.
    name: String,
    /// The folder it is in, as a person writes it (`~/Videos/Kerf`).
    place: String,
    exists: bool,
    branch: Option<String>,
    edited: Option<SystemTime>,
    /// How many presets its library holds.
    presets: usize,
}

impl Recent {
    /// What can be known at once: its folder, and whether it is there.
    fn quick(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            name: folder_of(path),
            place: path.parent().map(home_relative).unwrap_or_default(),
            exists: path.is_dir(),
            branch: None,
            edited: edited(path),
            presets: 0,
        }
    }

    /// Its document's name, its branch and its presets too, which read
    /// files and ask Git.
    fn inspected(path: &Path) -> Self {
        let mut recent = Self::quick(path);
        let info = kerf_bridge::project_inspect(&json!({"path": path.to_string_lossy()}));
        if let Some(name) = info["name"].as_str().filter(|name| !name.trim().is_empty()) {
            recent.name = name.to_owned();
        }
        recent.branch = info["branch"].as_str().map(str::to_owned);
        recent.presets = info["presets"].as_u64().unwrap_or(0) as usize;
        recent
    }

    /// What its row says on its right: "Edited 2 hours ago · 3 presets".
    fn meta(&self) -> String {
        if !self.exists {
            return "Not found".into();
        }
        let edited = match self.edited {
            Some(edited) => format!("Edited {}", ago(edited)),
            None => "No project yet".into(),
        };
        match self.presets {
            0 => edited,
            1 => format!("{edited} · 1 preset"),
            count => format!("{edited} · {count} presets"),
        }
    }
}

// ---------------------------------------------------------------- what Create makes

/// What Create asks for, as the form says it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub name: String,
    pub location: PathBuf,
    /// `standard`, `vertical` or `square`.
    pub template: String,
    pub rate: String,
    pub git: bool,
    /// A private GitHub repository, made with the GitHub CLI.
    pub github: bool,
    /// A Git address to push to; empty for none.
    pub remote: String,
    /// The kerfd the editor was started with.
    pub kerfd: Option<PathBuf>,
    /// `{path, bindings, texts?}` when it starts from a preset.
    pub preset: Option<Value>,
}

impl Plan {
    /// The bridge's `create` task for it: GitHub or a remote turn Git on,
    /// GitHub wins over a remote, and a kerfd named by its path is passed
    /// on (a bare name is looked up as the bridge would anyway).
    pub fn request(&self) -> Value {
        let remote = self.remote.trim();
        let mut request = json!({
            "type": "create",
            "name": self.name.trim(),
            "location": self.location.to_string_lossy(),
            "template": self.template,
            "rate": self.rate,
            "git": self.git || self.github || !remote.is_empty(),
        });
        if self.github {
            request["github"] = json!({"private": true});
        } else if !remote.is_empty() {
            request["remote"] = json!(remote);
        }
        if let Some(kerfd) = self
            .kerfd
            .as_ref()
            .filter(|kerfd| kerfd.components().count() > 1)
        {
            let kerfd = std::path::absolute(kerfd).unwrap_or_else(|_| kerfd.clone());
            request["kerfd"] = json!(kerfd.to_string_lossy());
        }
        if let Some(preset) = &self.preset {
            request["preset"] = preset.clone();
        }
        request
    }
}

/// Why a new project named `name` cannot be made in `location`, pushed to
/// `remote` (empty for none), in Qt's words; None when it can.
pub fn details_problem(name: &str, location: &Path, remote: &str) -> Option<String> {
    let folder = folder_name(name);
    if folder.is_empty() {
        return Some("Name the project".into());
    }
    if location.as_os_str().is_empty() {
        return Some("Choose where to save the project".into());
    }
    if !location.is_absolute() {
        return Some(format!(
            "Choose a full location, such as {}",
            home_relative(&default_videos())
        ));
    }
    if occupied(&location.join(&folder)) {
        return Some(format!(
            "“{folder}” is already in that location; choose another name"
        ));
    }
    let remote = remote.trim();
    if !remote.is_empty() && !git_address(remote) {
        return Some("The remote must be a Git address".into());
    }
    None
}

/// Why the repository at `url` cannot be cloned into `location`, in Qt's
/// words; None when it can.
pub fn clone_problem(url: &str, location: &Path) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return Some("Enter the repository's address".into());
    }
    let Some(folder) = repository_name(url).filter(|_| git_address(url)) else {
        return Some("Enter a Git address such as https://github.com/you/film.git".into());
    };
    if location.as_os_str().is_empty() || !location.is_absolute() {
        return Some("Choose where to put the copy".into());
    }
    if occupied(&location.join(&folder)) {
        return Some(format!("“{folder}” is already in that location"));
    }
    None
}

/// What the Sharing rows say of the tools the bridge found (`{git,
/// gitLfs, gh, ghUser}`), before they are found (None), or why they could
/// not be (`error`).
pub fn tools_hint(tools: Option<&Value>, error: Option<&str>) -> String {
    if let Some(error) = error {
        return format!("Kerf could not look for Git and the GitHub CLI: {error}");
    }
    let Some(tools) = tools else {
        return "Looking for Git and the GitHub CLI…".into();
    };
    if tools["git"] != true {
        return if cfg!(target_os = "macos") {
            "Install Git to share projects (xcode-select --install).".into()
        } else {
            "Install Git to share projects.".into()
        };
    }
    let mut hints = vec![match github_account(tools) {
        Some(user) => format!("Signed in to GitHub as @{user}."),
        None if tools["gh"] == true => {
            "Sign in with `gh auth login` to create a GitHub repository here.".to_owned()
        }
        None if cfg!(target_os = "macos") => {
            "Install the GitHub CLI (brew install gh) to create a GitHub repository here."
                .to_owned()
        }
        None => "Install the GitHub CLI (gh) to create a GitHub repository here.".to_owned(),
    }];
    if tools["gitLfs"] != true {
        hints.push("Git LFS is not installed: media of 100 MB or more cannot be shared.".into());
    }
    hints.join(" ")
}

/// The GitHub account the GitHub CLI is signed in to, if it is.
pub fn github_account(tools: &Value) -> Option<&str> {
    tools["ghUser"]
        .as_str()
        .filter(|user| tools["gh"] == true && !user.is_empty())
}

// ---------------------------------------------------------------- the keyboard

/// A text field of the launcher's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Name,
    Location,
    Url,
    Remote,
    Start(usize),
    Text(usize),
}

/// What the keyboard can be on, in the order Tab goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Stop {
    Settings,
    /// A way to begin; None opens a folder.
    Way(Option<Mode>),
    Field(Field),
    ChooseLocation,
    Format,
    Details,
    Rate,
    Git,
    GitHub,
    /// Create Project, or Clone.
    Act,
    PresetList,
    PresetFile,
    SlotFile(usize),
    Recent,
    ClearRecent,
}

/// Binds the launcher's keys, unless they are bound. Binding a keymap
/// clears them, so this runs again when Settings closes.
fn bind_keys(cx: &mut App) {
    let bound = Keystroke::parse("tab").is_ok_and(|tab| {
        cx.all_bindings_for_input(&[tab])
            .iter()
            .any(|binding| binding.action().partial_eq(&NextControl))
    });
    if bound {
        return;
    }
    cx.bind_keys([
        KeyBinding::new("tab", NextControl, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PreviousControl, Some(CONTEXT)),
        KeyBinding::new("enter", Choose, Some(CONTEXT)),
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("up", ChosenUp, Some(CONTEXT)),
        KeyBinding::new("down", ChosenDown, Some(CONTEXT)),
        KeyBinding::new("enter", Press, Some(CONTROL)),
        KeyBinding::new("space", Press, Some(CONTROL)),
        KeyBinding::new("left", Earlier, Some(CONTROL)),
        KeyBinding::new("right", Later, Some(CONTROL)),
    ]);
}

// ---------------------------------------------------------------- From Preset

/// A media slot of the preset being started from, and the file for it.
struct SlotFile {
    id: String,
    /// The slot as the preset describes it (`{id, seconds, video, audio, clips}`).
    slot: Value,
    file: Option<PathBuf>,
    fit: Option<Fit>,
    /// Where it starts in the file, in seconds.
    start: Entity<TextInput>,
}

/// A text of the preset, and its field.
struct PresetText {
    clip: String,
    original: String,
    input: Entity<TextInput>,
}

/// The From Preset page's choices.
#[derive(Default)]
struct PresetPage {
    /// The presets in the recent projects' `presets/`.
    choices: Vec<PresetChoice>,
    /// The one chosen among them; None for none, or a file chosen apart.
    chosen: Option<usize>,
    path: Option<PathBuf>,
    /// What the bridge read of it (`{name, slots, texts, made}` or
    /// `{error}`); None while it reads.
    preset: Option<Value>,
    slots: Vec<SlotFile>,
    texts: Vec<PresetText>,
    /// The chosen files are being checked against the slots.
    checking: bool,
    _reading: Option<Task<()>>,
    /// What its fields say, for the preset shown.
    _fields: Vec<Subscription>,
}

impl PresetPage {
    fn name(&self) -> Option<String> {
        let preset = self.preset.as_ref()?;
        preset["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.path
                    .as_ref()?
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
    }

    /// Why Create cannot make it yet, in Qt's words; None when the preset
    /// side is ready.
    fn problem(&self, cx: &App) -> Option<String> {
        let Some(preset) = &self.preset else {
            return Some(if self.path.is_some() {
                "Reading the preset…".into()
            } else {
                "Choose a preset".into()
            });
        };
        if preset["error"].is_string() {
            return Some("Choose a preset".into());
        }
        if self.slots.iter().any(|slot| slot.file.is_none()) {
            return Some("Choose a file for each media slot".into());
        }
        if self.checking {
            return Some("Checking the files…".into());
        }
        for slot in &self.slots {
            if !slot.fit.as_ref().is_some_and(|fit| fit.fits) {
                let file = slot
                    .file
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                return Some(format!("{file} cannot fill {}", slot.id));
            }
            // A start is asked for only while its field shows.
            if slot
                .fit
                .as_ref()
                .is_some_and(|fit| fit.start(slot.start.read(cx).text()).is_none())
            {
                return Some(format!("Give where {} starts in seconds", slot.id));
            }
        }
        None
    }

    /// The `preset` of the create request: each slot's file, where it
    /// starts when not at its beginning, and the texts changed.
    fn request(&self, cx: &App) -> Option<Value> {
        let path = self.path.as_ref()?;
        let mut bindings = Map::new();
        for slot in &self.slots {
            let mut binding = json!({"file": slot.file.as_ref()?.to_string_lossy()});
            let from = slot
                .fit
                .as_ref()
                .and_then(|fit| fit.start(slot.start.read(cx).text()))
                .unwrap_or(0.0);
            if from > 0.0 {
                binding["from"] = json!(format!("{from:.2}s"));
            }
            bindings.insert(slot.id.clone(), binding);
        }
        let mut request = json!({"path": path.to_string_lossy(), "bindings": bindings});
        let texts: Map<String, Value> = self
            .texts
            .iter()
            .filter(|text| text.input.read(cx).text() != text.original)
            .map(|text| (text.clip.clone(), json!(text.input.read(cx).text())))
            .collect();
        if !texts.is_empty() {
            request["texts"] = Value::Object(texts);
        }
        Some(request)
    }
}

// ---------------------------------------------------------------- the view

pub struct LauncherView {
    prefs: Prefs,
    /// The kerfd the editor was started with: a project made from a preset
    /// imports its media with it, and the window opened keeps it.
    kerfd: Option<PathBuf>,
    focus_handle: FocusHandle,
    mode: Option<Mode>,
    name: Entity<TextInput>,
    location: Entity<TextInput>,
    url: Entity<TextInput>,
    remote: Entity<TextInput>,
    format: Format,
    rate: &'static str,
    /// Frame rate and Sharing are shown.
    details: bool,
    git: bool,
    github: bool,
    /// What the bridge found to share with: `{git, gitLfs, gh, ghUser}`.
    tools: Option<Value>,
    tools_error: Option<String>,
    preset: PresetPage,
    recent: Vec<Recent>,
    /// The recent project Up and Down are on.
    recent_chosen: usize,
    /// What a running task says it is doing.
    busy: Option<SharedString>,
    /// Why the last thing asked could not be done.
    problem: Option<SharedString>,
    /// Settings while open over the launcher, and how many times it has
    /// been opened.
    settings: Option<(Entity<SettingsView>, u64)>,
    settings_opened: u64,
    /// What Settings works on here: a session with no project.
    session: Option<Entity<Session>>,
    /// The controls the keyboard goes to, each with its own focus.
    stops: HashMap<Stop, FocusHandle>,
    /// Their order as last drawn.
    order: Vec<(Stop, FocusHandle)>,
    _task: Option<Task<()>>,
    _tools: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LauncherEvent> for LauncherView {}

impl LauncherView {
    /// The launcher, as it was left (its way, template, frame rate and
    /// Details, from `prefs`), with the kerfd the editor was started with
    /// ([`crate::app::started_kerfd`]) unless [`Self::with_kerfd`] says
    /// another.
    pub fn new(prefs: Prefs, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let kerfd = crate::app::started_kerfd(cx);
        bind_keys(cx);
        let location = default_location(&prefs);
        let untitled = untitled(&location);
        let name = cx.new(|cx| {
            let mut input = TextInput::new(cx, "Untitled");
            input.set_text(untitled, cx);
            input
        });
        let location_input = cx.new(|cx| {
            let mut input = TextInput::new(cx, "~/Videos/Kerf");
            input.set_text(home_relative(&location), cx);
            input
        });
        let url = cx.new(|cx| TextInput::new(cx, "https://github.com/you/film.git"));
        let remote =
            cx.new(|cx| TextInput::new(cx, "Or push to a remote: git@github.com:you/film.git"));
        let mut subscriptions = Vec::new();
        for input in [&name, &location_input, &url, &remote] {
            subscriptions.push(cx.subscribe_in(input, window, Self::input_event));
            let handle = input.read(cx).focus_handle(cx);
            subscriptions.push(cx.on_focus(&handle, window, |_, _, cx| cx.notify()));
            subscriptions.push(cx.on_blur(&handle, window, |_, _, cx| cx.notify()));
        }
        // A remote typed turns sharing on.
        subscriptions.push(
            cx.subscribe(&remote, |this, remote, event: &InputEvent, cx| {
                if *event == InputEvent::Changed && !remote.read(cx).text().trim().is_empty() {
                    this.git = true;
                }
            }),
        );

        // The recent projects show at once by their folders; their names,
        // branches and presets follow, since finding them reads files and
        // asks Git.
        let recent = prefs
            .recent
            .iter()
            .map(|path| Recent::quick(path))
            .collect();
        let paths = prefs.recent.clone();
        let inspecting = cx.background_executor().spawn(async move {
            let recent = paths
                .iter()
                .map(|path| Recent::inspected(path))
                .collect::<Vec<_>>();
            let presets = presets::presets_in(&paths);
            (recent, presets)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (inspected, choices) = inspecting.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.recent = inspected
                    .into_iter()
                    .filter(|recent| this.prefs.recent.contains(&recent.path))
                    .collect();
                this.recent_chosen = this.recent_chosen.min(this.recent.len().saturating_sub(1));
                this.preset.choices = choices;
                // Left on From Preset: the first preset found is read at once.
                if this.mode == Some(Mode::Preset) && this.preset.path.is_none() {
                    this.load_choice(0, window, cx);
                }
                cx.notify();
            });
        })
        .detach();

        // What can share a project: Git, Git LFS, and a signed-in GitHub CLI.
        let tools = kerf_bridge::Task::start(json!({"type": "tools"}));
        let looking = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TASK_POLL).await;
            let status = tools.status();
            if status["status"] == "running" {
                continue;
            }
            let _ = this.update(cx, |this, cx| this.found_tools(status, cx));
            break;
        });

        let mode = Mode::from_wire(&prefs.launcher_way);
        let format = Format::ALL
            .into_iter()
            .find(|format| format.wire() == prefs.launcher_format)
            .unwrap_or(Format::Standard);
        let rate = RATES
            .iter()
            .map(|(rate, _)| *rate)
            .find(|rate| *rate == prefs.launcher_rate)
            .unwrap_or("30");
        let details = prefs.launcher_details;
        let focus_handle = cx.focus_handle();
        match mode {
            Some(Mode::New) => name.read(cx).focus(window),
            Some(Mode::Clone) => url.read(cx).focus(window),
            _ => window.focus(&focus_handle),
        }
        Self {
            prefs,
            kerfd,
            focus_handle,
            mode,
            name,
            location: location_input,
            url,
            remote,
            format,
            rate,
            details,
            git: false,
            github: false,
            tools: None,
            tools_error: None,
            preset: PresetPage::default(),
            recent,
            recent_chosen: 0,
            busy: None,
            problem: None,
            settings: None,
            settings_opened: 0,
            session: None,
            stops: HashMap::new(),
            order: Vec::new(),
            _task: None,
            _tools: Some(looking),
            _subscriptions: subscriptions,
        }
    }

    /// The kerfd a project made from a preset imports its media with, and
    /// the window opened keeps: the one the window that opened the launcher
    /// has. None keeps the one the editor was started with.
    pub fn with_kerfd(mut self, kerfd: Option<PathBuf>) -> Self {
        if kerfd.is_some() {
            self.kerfd = kerfd;
        }
        self
    }

    /// The kerfd it was given.
    pub fn kerfd(&self) -> Option<&Path> {
        self.kerfd.as_deref()
    }

    /// Says why something asked of the launcher could not be done (a
    /// kerf:// link that could not be followed).
    pub fn say(&mut self, problem: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.problem = Some(problem.into());
        cx.notify();
    }

    /// Opens Settings over the launcher on `page`, or goes to `page` in it.
    pub fn show_settings(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((settings, _)) = &self.settings {
            settings.update(cx, |settings, cx| settings.show_page(page, cx));
            return;
        }
        let session = match &self.session {
            Some(session) => session.clone(),
            None => {
                let session =
                    Session::open(None, self.kerfd.clone(), self.prefs.clone(), window, cx);
                self.session = Some(session.clone());
                session
            }
        };
        let settings = cx.new(|cx| SettingsView::new_at(session, page, window, cx));
        cx.subscribe_in(
            &settings,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Dismiss => this.close_settings(window, cx),
            },
        )
        .detach();
        window.focus(&settings.focus_handle(cx));
        self.settings_opened += 1;
        self.settings = Some((settings, self.settings_opened));
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings = None;
        // What Settings saved, for the projects listed here; a keymap
        // chosen there cleared the launcher's keys.
        self.prefs.refresh();
        bind_keys(cx);
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// The window's commands that mean something with no project.
    fn command(&mut self, action: &RunCommand, window: &mut Window, cx: &mut Context<Self>) {
        match action.id.as_ref() {
            "app.preferences" if self.settings.is_some() => self.close_settings(window, cx),
            "app.preferences" => self.show_settings(Page::Chat, window, cx),
            "app.account" => self.show_settings(Page::Account, window, cx),
            "help.about" => self.show_settings(Page::About, window, cx),
            "app.quit" => cx.quit(),
            "file.openProject" => self.open_folder(window, cx),
            "file.newProject" if self.mode != Some(Mode::New) => {
                self.choose_mode(Mode::New, window, cx)
            }
            _ => {}
        }
    }

    /// Settings over the dimmed launcher, rising into its middle as it
    /// does over a project's window.
    /// Settings over the launcher, placed as the workspace places it
    /// ([`ui::modal_bounds`]): at most its size, else the window less a margin.
    fn settings_over(&self, window: &Window) -> Option<gpui::AnyElement> {
        let (settings, opened) = self.settings.as_ref()?;
        let (width, height) = crate::settings::MOST;
        let placed = ui::modal_bounds(gpui::size(px(width), px(height)), window.viewport_size());
        Some(
            div()
                .id("launcher-scrim")
                .absolute()
                .inset_0()
                .bg(theme::with_alpha(gpui::rgb(0x000000), theme::motion::SCRIM))
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .absolute()
                        .left(placed.origin.x)
                        .top(placed.origin.y)
                        .w(placed.size.width)
                        .h(placed.size.height)
                        .child(
                            div()
                                .relative()
                                .size_full()
                                .rounded(px(radius::PANEL))
                                .overflow_hidden()
                                .border_1()
                                .border_color(theme::line2())
                                .shadow_2xl()
                                .child(settings.clone())
                                .with_animation(
                                    ("launcher-settings-rise", *opened),
                                    motion::animation(theme::motion::PAGE),
                                    |body, t| {
                                        body.top(px(theme::motion::PAGE_RISE * (1.0 - t)))
                                            .opacity(0.2 + 0.8 * t)
                                    },
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn input_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => {
                self.problem = None;
                cx.notify();
            }
            InputEvent::Submit => self.choose(window, cx),
            InputEvent::Cancel => self.back(window, cx),
        }
    }

    /// Esc: the form goes, and the keyboard is on the launcher again.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_none() && self.mode.is_some() {
            self.mode = None;
            self.problem = None;
        }
        window.focus(&self.focus_handle);
        cx.notify();
    }

    fn choose_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.problem = None;
        if self.mode == Some(mode) {
            self.mode = None;
            window.focus(&self.focus_handle);
        } else {
            self.mode = Some(mode);
            match mode {
                Mode::New => self.name.read(cx).focus(window),
                Mode::Clone => self.url.read(cx).focus(window),
                Mode::Preset => {
                    // The first preset found is read at once.
                    if self.preset.path.is_none() && !self.preset.choices.is_empty() {
                        self.load_choice(0, window, cx);
                    }
                    if let Some(handle) = self.stops.get(&Stop::Way(Some(Mode::Preset))) {
                        window.focus(handle);
                    }
                }
            }
        }
        cx.notify();
    }

    /// Return: what the page is for. Create or Clone when the form can;
    /// with no form, the recent project chosen opens.
    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        match self.mode {
            Some(Mode::New) | Some(Mode::Preset) => self.create(cx),
            Some(Mode::Clone) => self.clone_repository(cx),
            None => self.open_chosen(window, cx),
        }
    }

    /// Keeps how the launcher was left, for the next time it opens.
    fn remember(&mut self, way: Option<Mode>) {
        self.prefs.refresh();
        self.prefs.launcher_way = way.map(Mode::wire).unwrap_or_default().to_owned();
        self.prefs.launcher_format = self.format.wire().to_owned();
        self.prefs.launcher_rate = self.rate.to_owned();
        self.prefs.launcher_details = self.details;
        self.prefs.save();
    }

    /// Opens `project`; `starts` unless said is whether it is no project
    /// yet. `warnings` are said in its window once it is open.
    fn open(
        &mut self,
        project: PathBuf,
        starts: Option<bool>,
        warnings: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let starts = starts.unwrap_or_else(|| crate::workspace::needs_start(&project));
        crate::app::warn_when_open(&project, warnings, cx);
        cx.emit(LauncherEvent::Open { project, starts });
    }

    fn open_chosen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(recent) = self.recent.get(self.recent_chosen) else {
            return;
        };
        if !recent.exists {
            self.problem = Some("That project folder is no longer there".into());
            cx.notify();
            return;
        }
        let path = recent.path.clone();
        self.remember(None);
        self.open(path, Some(false), Vec::new(), cx);
        window.focus(&self.focus_handle);
    }

    /// Open Folder…: a project opens; a folder that is no project yet is
    /// asked about first, and once started its media comes in.
    fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let chosen = presets::choose_paths(
            cx,
            PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Open".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let folder = match chosen.await {
                Picked::Paths(paths) => paths.into_iter().next(),
                Picked::Failed(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.say(format!("No folder chooser could open: {error}"), cx)
                    });
                    None
                }
                Picked::Unusable(words) => {
                    let _ = this.update(cx, |this, cx| this.say(words, cx));
                    None
                }
                Picked::Nothing => None,
            };
            let Some(folder) = folder else {
                return;
            };
            let Ok(asked) = this.update_in(cx, |this, window, cx| {
                this.ask_to_start(folder.clone(), window, cx)
            }) else {
                return;
            };
            let starts = match asked {
                None => false,
                Some(answer) => match answer.await {
                    Ok(0) => true,
                    _ => return,
                },
            };
            let _ = this.update(cx, |this, cx| {
                this.open(folder, Some(starts), Vec::new(), cx)
            });
        })
        .detach();
    }

    /// "“X” is not a project yet. Start one here, with its media?" for a
    /// folder with no project.kerf: Start (0) or Cancel. None for a project.
    fn ask_to_start(
        &mut self,
        folder: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl std::future::Future<Output = Result<usize, impl Sized>>> {
        if !crate::workspace::needs_start(&folder) {
            return None;
        }
        Some(window.prompt(
            PromptLevel::Info,
            &format!("“{}” is not a project yet.", folder_of(&folder)),
            Some("Start one here, with its media?"),
            &[PromptButton::ok("Start"), PromptButton::cancel("Cancel")],
            cx,
        ))
    }

    fn choose_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = presets::choose_paths(
            cx,
            PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Choose".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let chosen = chosen.await;
            let _ = this.update(cx, |this, cx| match chosen {
                Picked::Paths(paths) => {
                    if let Some(folder) = paths.into_iter().next() {
                        this.location
                            .update(cx, |input, cx| input.set_text(home_relative(&folder), cx));
                    }
                }
                Picked::Failed(error) => {
                    this.problem = Some(
                        format!("No folder chooser could open: {error}; type the folder instead")
                            .into(),
                    );
                    cx.notify();
                }
                Picked::Unusable(words) => {
                    this.problem = Some(words.into());
                    cx.notify();
                }
                Picked::Nothing => {}
            });
        })
        .detach();
    }

    /// Where the project or the clone goes, as typed.
    fn location(&self, cx: &App) -> PathBuf {
        expanded(self.location.read(cx).text().trim())
    }

    /// Why Return cannot do what the page is for, in Qt's words, as it is
    /// typed; None when it can.
    fn form_problem(&self, cx: &App) -> Option<String> {
        let details = || {
            details_problem(
                self.name.read(cx).text(),
                &self.location(cx),
                if self.github {
                    ""
                } else {
                    self.remote.read(cx).text()
                },
            )
        };
        match self.mode? {
            Mode::New => details(),
            Mode::Preset => self.preset.problem(cx).or_else(details),
            Mode::Clone => clone_problem(self.url.read(cx).text(), &self.location(cx)),
        }
    }

    fn plan(&self, cx: &App) -> Plan {
        Plan {
            name: self.name.read(cx).text().trim().to_owned(),
            location: self.location(cx),
            template: self.format.wire().to_owned(),
            rate: self.rate.to_owned(),
            git: self.git,
            github: self.github,
            remote: if self.github {
                String::new()
            } else {
                self.remote.read(cx).text().trim().to_owned()
            },
            kerfd: self.kerfd.clone(),
            preset: (self.mode == Some(Mode::Preset))
                .then(|| self.preset.request(cx))
                .flatten(),
        }
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.form_problem(cx).is_some() {
            return;
        }
        let plan = self.plan(cx);
        let folder = folder_name(&plan.name);
        let message = if plan.preset.is_some() {
            format!("Making {folder} from the preset…")
        } else {
            format!("Making {folder}…")
        };
        self.remember(self.mode);
        self.run(plan.request(), message, true, cx);
    }

    fn clone_repository(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.form_problem(cx).is_some() {
            return;
        }
        let url = self.url.read(cx).text().trim().to_owned();
        let folder = repository_name(&url).unwrap_or_default();
        let request =
            json!({"type": "clone", "url": url, "location": self.location(cx).to_string_lossy()});
        self.remember(Some(Mode::Clone));
        self.run(request, format!("Cloning {folder}…"), false, cx);
    }

    /// Starts a project task and follows it until it ends; `made` when it
    /// makes a project from nothing.
    fn run(&mut self, request: Value, message: String, made: bool, cx: &mut Context<Self>) {
        self.busy = Some(message.into());
        self.problem = None;
        let task = kerf_bridge::Task::start(request);
        self._task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TASK_POLL).await;
            let status = task.status();
            let running = status["status"] == "running";
            let followed = this.update(cx, |this, cx| this.task_status(&status, made, cx));
            if followed.is_err() || !running {
                break;
            }
        }));
        cx.notify();
    }

    fn task_status(&mut self, status: &Value, made: bool, cx: &mut Context<Self>) {
        match status["status"].as_str() {
            Some("running") => {
                if let Some(message) = status["message"]
                    .as_str()
                    .filter(|message| !message.is_empty())
                {
                    self.busy = Some(message.to_owned().into());
                }
            }
            Some("done") => {
                self.busy = None;
                // Said once the project is open: this window closes at once.
                let warnings: Vec<String> = status["warnings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                match status["path"].as_str().filter(|path| !path.is_empty()) {
                    Some(path) => {
                        self.open(PathBuf::from(path), made.then_some(true), warnings, cx)
                    }
                    None => {
                        self.problem = Some("The task finished without a project folder".into())
                    }
                }
            }
            _ => {
                self.busy = None;
                let error = status["error"]
                    .as_str()
                    .unwrap_or("Kerf could not make the project");
                self.problem = Some(error.to_owned().into());
            }
        }
        cx.notify();
    }

    fn found_tools(&mut self, status: Value, cx: &mut Context<Self>) {
        if status["status"] == "done" {
            if status["git"] != true {
                self.git = false;
            }
            if github_account(&status).is_none() {
                self.github = false;
            }
            self.tools = Some(status);
        } else {
            // Sharing still works when Git is there; the task says if not.
            self.tools_error = Some(
                status["error"]
                    .as_str()
                    .unwrap_or("the task stopped")
                    .to_owned(),
            );
        }
        cx.notify();
    }

    fn git_found(&self) -> bool {
        self.tools.as_ref().is_none_or(|tools| tools["git"] == true)
    }

    fn set_git(&mut self, on: bool, cx: &mut Context<Self>) {
        if on && !self.git_found() {
            return;
        }
        self.git = on;
        // Without Git there is nothing to publish.
        if !on {
            self.github = false;
            self.remote.update(cx, |remote, cx| remote.clear(cx));
            self.git = false;
        }
        cx.notify();
    }

    fn set_github(&mut self, on: bool, cx: &mut Context<Self>) {
        let signed_in = self.tools.as_ref().and_then(github_account).is_some();
        if on && !signed_in {
            return;
        }
        self.github = on;
        if on {
            self.git = true;
        }
        cx.notify();
    }

    fn set_details(&mut self, open: bool, cx: &mut Context<Self>) {
        self.details = open;
        self.prefs.refresh();
        self.prefs.launcher_details = open;
        self.prefs.save();
        cx.notify();
    }

    /// Takes a project that is no longer there out of the recent ones.
    fn forget(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.prefs.refresh();
        self.prefs.recent.retain(|known| known != path);
        self.prefs.save();
        self.recent.retain(|recent| recent.path != path);
        self.recent_chosen = self.recent_chosen.min(self.recent.len().saturating_sub(1));
        cx.notify();
    }

    /// Clears the recent projects, here and in every window's settings.
    fn clear_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.refresh();
        self.prefs.recent.clear();
        self.prefs.save();
        self.recent.clear();
        self.recent_chosen = 0;
        if self
            .order
            .iter()
            .any(|(stop, handle)| *stop == Stop::ClearRecent && handle.is_focused(window))
        {
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    // ------------------------------------------------------------ From Preset

    fn load_choice(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.preset.choices.get(index) else {
            return;
        };
        let path = choice.path.clone();
        self.preset.chosen = Some(index);
        self.load_preset(path, window, cx);
    }

    /// Reads the preset at `path` (on a thread of its own: it may probe
    /// files), and shows its slots, what it makes and its texts.
    fn load_preset(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.preset.path.as_ref() == Some(&path) && self.preset.preset.is_some() {
            return;
        }
        self.preset.path = Some(path.clone());
        self.preset.preset = None;
        self.preset.slots.clear();
        self.preset.texts.clear();
        self.preset.checking = false;
        let asked = path.to_string_lossy().into_owned();
        let reading = cx
            .background_executor()
            .spawn(async move { kerf_bridge::preset_inspect(&json!({"path": asked})) });
        self.preset._reading = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = reading.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.preset.path.as_ref() == Some(&path) {
                    this.show_preset(answer, window, cx);
                }
            });
        }));
        cx.notify();
    }

    fn show_preset(&mut self, preset: Value, window: &mut Window, cx: &mut Context<Self>) {
        let mut fields = Vec::new();
        let mut slots = Vec::new();
        for slot in preset["slots"].as_array().into_iter().flatten() {
            let start = cx.new(|cx| {
                let mut input = TextInput::new(cx, "0");
                input.set_text("0", cx);
                input
            });
            fields.push(cx.subscribe_in(&start, window, Self::preset_field));
            slots.push(SlotFile {
                id: slot["id"].as_str().unwrap_or_default().to_owned(),
                slot: slot.clone(),
                file: None,
                fit: None,
                start,
            });
        }
        let mut texts = Vec::new();
        for (clip, original) in preset["texts"].as_object().into_iter().flatten() {
            let original = original.as_str().unwrap_or_default().to_owned();
            let words = original.clone();
            let input = cx.new(|cx| {
                let mut input = TextInput::new(cx, "Text");
                input.set_text(words, cx);
                input
            });
            fields.push(cx.subscribe_in(&input, window, Self::preset_field));
            texts.push(PresetText {
                clip: clip.clone(),
                original,
                input,
            });
        }
        self.preset.slots = slots;
        self.preset.texts = texts;
        self.preset._fields = fields;
        self.preset.preset = Some(preset);
        cx.notify();
    }

    /// The From Preset page's own fields answer as the form's others do:
    /// Return makes the project, Esc puts the form away.
    fn preset_field(
        &mut self,
        _: &Entity<TextInput>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Changed => cx.notify(),
            InputEvent::Submit => self.create(cx),
            InputEvent::Cancel => self.back(window, cx),
        }
    }

    fn choose_preset_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let _ = this.update_in(cx, |this, window, cx| match answer {
                Picked::Paths(paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.preset.chosen = None;
                        this.load_preset(path, window, cx);
                    }
                }
                Picked::Failed(error) => {
                    this.say(format!("No file chooser could open: {error}"), cx)
                }
                Picked::Unusable(words) => this.say(words, cx),
                Picked::Nothing => {}
            });
        })
        .detach();
    }

    fn choose_slot_file(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose".into()),
            },
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let _ = this.update(cx, |this, cx| match answer {
                Picked::Paths(paths) => {
                    if let (Some(file), Some(entry)) =
                        (paths.into_iter().next(), this.preset.slots.get_mut(slot))
                    {
                        entry.file = Some(file);
                        entry.fit = None;
                        this.refit(cx);
                    }
                }
                Picked::Failed(error) => {
                    this.say(format!("No file chooser could open: {error}"), cx)
                }
                Picked::Unusable(words) => this.say(words, cx),
                Picked::Nothing => {}
            });
        })
        .detach();
    }

    /// Checks the files chosen against the slots, each once.
    fn refit(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preset.path.clone() else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        let mut files: Vec<PathBuf> = Vec::new();
        for file in self
            .preset
            .slots
            .iter()
            .filter_map(|slot| slot.file.clone())
        {
            if !files.contains(&file) {
                files.push(file);
            }
        }
        if files.is_empty() {
            return;
        }
        self.preset.checking = true;
        let asked: Vec<String> = files
            .iter()
            .map(|file| file.to_string_lossy().into_owned())
            .collect();
        let checking = cx.background_executor().spawn(async move {
            kerf_bridge::preset_inspect(&json!({"path": path, "files": asked}))
        });
        self.preset._reading = Some(cx.spawn(async move |this, cx| {
            let answer = checking.await;
            let _ = this.update(cx, |this, cx| {
                this.preset.checking = false;
                for slot in &mut this.preset.slots {
                    if let Some(file) = &slot.file {
                        slot.fit = Some(presets::file_fit(&answer, &files, &slot.slot, file));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    // ------------------------------------------------------------ the keyboard

    /// The controls on screen, in the order Tab goes.
    fn stop_order(&self) -> Vec<Stop> {
        let mut order = vec![
            Stop::Settings,
            Stop::Way(Some(Mode::New)),
            Stop::Way(Some(Mode::Preset)),
            Stop::Way(None),
            Stop::Way(Some(Mode::Clone)),
        ];
        let details = |order: &mut Vec<Stop>| {
            order.extend([
                Stop::Field(Field::Name),
                Stop::Field(Field::Location),
                Stop::ChooseLocation,
            ]);
        };
        // Only what can be used: a switch with nothing to share with, or
        // the remote while GitHub is chosen, is passed over.
        let git_found = self.git_found();
        let signed_in = self.tools.as_ref().and_then(github_account).is_some();
        let github = self.github;
        let sharing = |order: &mut Vec<Stop>, open: bool| {
            order.push(Stop::Details);
            if open {
                order.push(Stop::Rate);
                if git_found {
                    order.push(Stop::Git);
                }
                if signed_in {
                    order.push(Stop::GitHub);
                }
                if git_found && !github {
                    order.push(Stop::Field(Field::Remote));
                }
            }
        };
        match self.mode {
            Some(Mode::New) => {
                details(&mut order);
                order.push(Stop::Format);
                sharing(&mut order, self.details);
                order.push(Stop::Act);
            }
            Some(Mode::Preset) => {
                if !self.preset.choices.is_empty() {
                    order.push(Stop::PresetList);
                }
                order.push(Stop::PresetFile);
                for (index, slot) in self.preset.slots.iter().enumerate() {
                    if slot.fit.as_ref().is_some_and(Fit::start_open) {
                        order.push(Stop::Field(Field::Start(index)));
                    }
                    order.push(Stop::SlotFile(index));
                }
                order.extend(
                    (0..self.preset.texts.len()).map(|index| Stop::Field(Field::Text(index))),
                );
                details(&mut order);
                sharing(&mut order, self.details);
                order.push(Stop::Act);
            }
            Some(Mode::Clone) => {
                order.extend([
                    Stop::Field(Field::Url),
                    Stop::Field(Field::Location),
                    Stop::ChooseLocation,
                    Stop::Act,
                ]);
            }
            None => {}
        }
        if !self.recent.is_empty() {
            order.extend([Stop::Recent, Stop::ClearRecent]);
        }
        order
    }

    fn field_input(&self, field: Field) -> Option<&Entity<TextInput>> {
        match field {
            Field::Name => Some(&self.name),
            Field::Location => Some(&self.location),
            Field::Url => Some(&self.url),
            Field::Remote => Some(&self.remote),
            Field::Start(index) => self.preset.slots.get(index).map(|slot| &slot.start),
            Field::Text(index) => self.preset.texts.get(index).map(|text| &text.input),
        }
    }

    /// Lays out the order Tab goes in, each control with its focus.
    fn arrange(&mut self, cx: &mut Context<Self>) {
        let mut order = Vec::new();
        for stop in self.stop_order() {
            let handle = match stop {
                Stop::Field(field) => match self.field_input(field) {
                    Some(input) => input.read(cx).focus_handle(cx),
                    None => continue,
                },
                stop => self
                    .stops
                    .entry(stop)
                    .or_insert_with(|| cx.focus_handle())
                    .clone(),
            };
            order.push((stop, handle));
        }
        self.order = order;
    }

    fn focused(&self, window: &Window) -> Option<Stop> {
        self.order
            .iter()
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(stop, _)| *stop)
    }

    fn handle(&self, stop: Stop) -> Option<FocusHandle> {
        self.order
            .iter()
            .find(|(known, _)| *known == stop)
            .map(|(_, handle)| handle.clone())
    }

    /// Tab and Shift-Tab: the next control, or the one before, round.
    fn step_focus(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.order.is_empty() {
            return;
        }
        let count = self.order.len() as isize;
        let next = match self
            .order
            .iter()
            .position(|(_, handle)| handle.is_focused(window))
        {
            Some(at) => (at as isize + by).rem_euclid(count),
            None if by > 0 => 0,
            None => count - 1,
        };
        let (_, handle) = &self.order[next as usize];
        window.focus(handle);
        cx.notify();
    }

    /// Up and Down: the recent project chosen, or the preset.
    fn step_chosen(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused(window) {
            Some(Stop::PresetList) if !self.preset.choices.is_empty() => {
                let last = self.preset.choices.len() as isize - 1;
                let at = self
                    .preset
                    .chosen
                    .map_or(0, |at| (at as isize + by).clamp(0, last) as usize);
                self.load_choice(at, window, cx);
            }
            Some(Stop::Recent) if !self.recent.is_empty() => {
                let last = self.recent.len() as isize - 1;
                self.recent_chosen = (self.recent_chosen as isize + by).clamp(0, last) as usize;
                cx.notify();
            }
            // Coming to the list, the arrow shows the project chosen first.
            Some(Stop::ClearRecent) | None if !self.recent.is_empty() => {
                if let Some(handle) = self.handle(Stop::Recent) {
                    window.focus(&handle);
                }
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    /// Space or Return on a control: what pressing it does.
    fn press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stop) = self.focused(window) else {
            return;
        };
        match stop {
            Stop::Settings => self.show_settings(Page::Chat, window, cx),
            // A way is pressed by its own click, which GPUI gives a focused
            // element on Space and Return.
            Stop::Way(_) => {}
            Stop::ChooseLocation => self.choose_location(window, cx),
            Stop::Format | Stop::Rate => self.cycle(stop, 1, cx),
            Stop::Details => self.set_details(!self.details, cx),
            Stop::Git => self.set_git(!self.git, cx),
            Stop::GitHub => self.set_github(!self.github, cx),
            Stop::Act => self.choose(window, cx),
            Stop::PresetList => {
                let at = self.preset.chosen.unwrap_or(0);
                self.load_choice(at, window, cx);
            }
            Stop::PresetFile => self.choose_preset_file(window, cx),
            Stop::SlotFile(slot) => self.choose_slot_file(slot, window, cx),
            Stop::Recent => self.open_chosen(window, cx),
            Stop::ClearRecent => self.clear_recent(window, cx),
            Stop::Field(_) => {}
        }
    }

    /// Left and Right on the template or the frame rate.
    fn cycle(&mut self, stop: Stop, by: isize, cx: &mut Context<Self>) {
        let step = |at: usize, count: usize| (at as isize + by).rem_euclid(count as isize) as usize;
        match stop {
            Stop::Format => {
                let at = Format::ALL
                    .iter()
                    .position(|f| *f == self.format)
                    .unwrap_or(0);
                self.format = Format::ALL[step(at, Format::ALL.len())];
            }
            Stop::Rate => {
                let at = RATES.iter().position(|(r, _)| *r == self.rate).unwrap_or(0);
                self.rate = RATES[step(at, RATES.len())].0;
            }
            _ => return,
        }
        cx.notify();
    }

    fn arrow(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused(window) {
            Some(stop @ (Stop::Format | Stop::Rate)) => self.cycle(stop, by, cx),
            _ => cx.propagate(),
        }
    }

    /// `control` with the ring the keyboard shows on it, taking Space and
    /// Return when it is on it.
    fn ring(
        &self,
        stop: Stop,
        window: &Window,
        control: impl IntoElement,
    ) -> gpui::Stateful<gpui::Div> {
        let handle = self.handle(stop);
        let focused = handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        let mut ring = div()
            .id(ElementId::Name(format!("launcher-ring-{stop:?}").into()))
            .key_context(CONTROL)
            .flex_none()
            .p(px(RING))
            .rounded(px(radius::CONTROL + RING))
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::with_alpha(theme::accent(), 0.0)
            })
            .child(control);
        if let Some(handle) = &handle {
            ring = ring.track_focus(handle);
        }
        ring
    }

    // ------------------------------------------------------------ drawing

    fn way(
        &self,
        mode: Option<Mode>,
        icon: Icon,
        title: &'static str,
        hint: &'static str,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let on = mode.is_some() && mode == self.mode;
        let idle = self.busy.is_none();
        let handle = self.handle(Stop::Way(mode));
        let focused = handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        let mut way = div()
            .id(title)
            .key_context(CONTROL)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(theme::space::S6))
            .p(px(theme::space::S16))
            .rounded(px(radius::CARD))
            .bg(if on || focused {
                theme::card()
            } else {
                theme::chrome()
            })
            .border_1()
            .border_color(if on || focused {
                theme::accent()
            } else {
                theme::line()
            })
            .when(idle, |way| {
                way.cursor_pointer().hover(|style| {
                    style.bg(theme::card()).border_color(if on {
                        theme::accent()
                    } else {
                        theme::lineh()
                    })
                })
            })
            .child(icon.view(
                20.0,
                if on {
                    theme::accent_ink()
                } else {
                    theme::ink2()
                },
            ))
            .child(ui::label(
                title,
                text::TITLE,
                if idle { theme::ink() } else { theme::dis() },
            ))
            .child(ui::label(hint, text::CONTROL, theme::ink3()))
            .on_click(cx.listener(move |this, _, window, cx| match mode {
                Some(mode) => this.choose_mode(mode, window, cx),
                None => this.open_folder(window, cx),
            }));
        if let Some(handle) = &handle {
            way = way.track_focus(handle);
        }
        way
    }

    fn location_row(&self, window: &Window, cx: &Context<Self>) -> gpui::Div {
        let idle = self.busy.is_none();
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .child(field(&self.location, window, cx).flex_1())
            .child(
                self.ring(
                    Stop::ChooseLocation,
                    window,
                    ui::button("launcher-location", "Choose…")
                        .disabled(!idle)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.choose_location(window, cx)),
                        ),
                ),
            )
    }

    /// What Create makes, in one line: the name, what it starts from, the
    /// frame rate, where, and how it is shared.
    fn summary(&self, cx: &App) -> String {
        let name = self.name.read(cx).text().trim().to_owned();
        let mut summary = vec![if name.is_empty() {
            "Create an unnamed project".to_owned()
        } else {
            format!("Create “{name}”")
        }];
        if self.mode == Some(Mode::Preset) {
            summary.push(match self.preset.name() {
                Some(preset) => format!("from {preset}"),
                None => "from a preset".into(),
            });
        } else {
            summary.push(self.format.label().to_owned());
        }
        summary.push(format!("{} fps", self.rate));
        summary.push(home_relative(&self.location(cx)));
        if self.github {
            summary.push("private GitHub repository".into());
        } else if !self.remote.read(cx).text().trim().is_empty() {
            summary.push("Git, pushed to the remote".into());
        } else if self.git {
            summary.push("Git".into());
        }
        summary.join(" · ")
    }

    /// The Details disclosure: the frame rate and how the project is shared.
    fn details_rows(&self, window: &Window, cx: &Context<Self>) -> Vec<gpui::AnyElement> {
        let toggle = div()
            .id("launcher-details")
            .flex()
            .items_center()
            .gap(px(theme::space::S4))
            .cursor_pointer()
            .child(
                if self.details {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                }
                .view(12.0, theme::ink2()),
            )
            .child(ui::label("Details", text::CONTROL, theme::ink2()))
            .on_click(cx.listener(|this, _, _, cx| this.set_details(!this.details, cx)));
        let mut rows = vec![div()
            .flex()
            .items_center()
            .gap(px(theme::space::S12))
            .child(
                self.ring(Stop::Details, window, toggle)
                    .w(px(FORM_LABEL))
                    .flex_none(),
            )
            .when(!self.details, |row| {
                row.child(
                    ui::line(self.summary(cx), text::CONTROL, theme::ink3())
                        .flex_1()
                        .min_w_0(),
                )
            })
            .into_any_element()];
        if !self.details {
            return rows;
        }
        let signed_in = self
            .tools
            .as_ref()
            .and_then(github_account)
            .map(str::to_owned);
        let git_found = self.git_found();
        let remote_open = git_found && !self.github;
        let switch_row =
            |stop: Stop,
             id: &'static str,
             on: bool,
             enabled: bool,
             words: String,
             toggle: fn(&mut Self, bool, &mut Context<Self>)| {
                let this = cx.entity().downgrade();
                let control = ui::switch(id, on, move |on, _, cx| {
                    if enabled {
                        let _ = this.update(cx, |this, cx| toggle(this, on, cx));
                    }
                });
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S12))
                    .child(
                        self.ring(stop, window, control)
                            .when(!enabled, |ring| ring.opacity(0.4)),
                    )
                    .child(ui::label(
                        words,
                        text::CONTROL,
                        if enabled { theme::ink() } else { theme::dis() },
                    ))
            };
        let remote = if remote_open {
            field(&self.remote, window, cx).flex_1().into_any_element()
        } else {
            div()
                .flex_1()
                .h(px(height::H28))
                .flex()
                .items_center()
                .px(px(theme::space::S8))
                .rounded(px(radius::CONTROL))
                .border_1()
                .border_color(theme::line())
                .child(ui::line(
                    "Or push to a remote: git@github.com:you/film.git",
                    text::BODY,
                    theme::dis(),
                ))
                .into_any_element()
        };
        rows.push(form_row(
            "Frame rate",
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .child(self.ring(
                    Stop::Rate,
                    window,
                    ui::segmented(
                        "launcher-rate",
                        &RATES,
                        self.rate,
                        Self::chooser(cx, |this, rate, _, cx| {
                            this.rate = rate;
                            cx.notify();
                        }),
                    ),
                ))
                .child(ui::label("fps", text::CONTROL, theme::ink3())),
        ));
        rows.push(form_row(
            "Sharing",
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(theme::space::S6))
                .child(switch_row(
                    Stop::Git,
                    "launcher-git",
                    self.git,
                    git_found,
                    "Share with Git, so collaborators pull, merge and push its edits".into(),
                    |this, on, cx| this.set_git(on, cx),
                ))
                .child(switch_row(
                    Stop::GitHub,
                    "launcher-github",
                    self.github,
                    signed_in.is_some(),
                    match &signed_in {
                        Some(user) => format!("Create a private GitHub repository as @{user}"),
                        None => "Create a private GitHub repository".into(),
                    },
                    |this, on, cx| this.set_github(on, cx),
                ))
                .child(div().flex().child(remote))
                .child(ui::label(
                    tools_hint(self.tools.as_ref(), self.tools_error.as_deref()),
                    text::CONTROL,
                    theme::ink3(),
                )),
        ));
        rows.into_iter()
            .enumerate()
            .map(|(index, row)| {
                if index == 0 {
                    row
                } else {
                    div()
                        .child(row)
                        .with_animation(
                            ElementId::named_usize("launcher-details-in", index),
                            motion::animation(theme::motion::APPEAR),
                            |row, t| row.opacity(t),
                        )
                        .into_any_element()
                }
            })
            .collect()
    }

    /// The From Preset page: the presets found, the one chosen, a file for
    /// each of its slots, what it makes and its texts.
    fn preset_rows(&self, window: &Window, cx: &Context<Self>) -> Vec<gpui::AnyElement> {
        let page = &self.preset;
        let idle = self.busy.is_none();
        let mut rows = Vec::new();
        rows.push(
            div()
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .child(
                    ui::label("Presets in your projects", text::META_STRONG, theme::ink3())
                        .flex_1(),
                )
                .child(
                    self.ring(
                        Stop::PresetFile,
                        window,
                        ui::button("launcher-preset-file", "Choose a Preset File…")
                            .small()
                            .disabled(!idle)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_preset_file(window, cx)
                            })),
                    ),
                )
                .into_any_element(),
        );
        let list_focused = self
            .handle(Stop::PresetList)
            .is_some_and(|handle| handle.is_focused(window));
        let mut list = div()
            .id("launcher-preset-list")
            .key_context(CONTROL)
            .flex()
            .flex_col()
            .rounded(px(radius::CARD))
            .border_1()
            .border_color(if list_focused {
                theme::accent()
            } else {
                theme::line()
            })
            .overflow_hidden();
        if let Some(handle) = self.handle(Stop::PresetList) {
            list = list.track_focus(&handle);
        }
        if page.choices.is_empty() {
            list = list.child(
                ui::label(
                    "Presets saved in your projects' presets/ folders appear here.",
                    text::CONTROL,
                    theme::ink3(),
                )
                .p(px(theme::space::S12)),
            );
        }
        list = list.children(page.choices.iter().enumerate().map(|(index, choice)| {
            let chosen = page.chosen == Some(index);
            div()
                .id(ElementId::named_usize("launcher-preset", index))
                .h(px(PRESET_ROW))
                .px(px(theme::space::S12))
                .flex()
                .flex_col()
                .justify_center()
                .when(index > 0, |row| {
                    row.border_t_1().border_color(theme::line())
                })
                .bg(if chosen {
                    theme::accent_wash()
                } else {
                    theme::with_alpha(theme::chrome(), 0.0)
                })
                .cursor_pointer()
                .hover(|style| style.bg(theme::card()))
                .relative()
                .child(ui::tip_zone(
                    ElementId::named_usize("launcher-preset-tip", index),
                    Some(ui::Tip::new(choice.path.display().to_string())),
                ))
                .child(ui::line(choice.name.clone(), text::BODY_NAME, theme::ink()))
                .child(ui::line(
                    format!("from {}", choice.project),
                    text::META,
                    theme::ink3(),
                ))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.load_choice(index, window, cx)),
                )
        }));
        rows.push(list.into_any_element());

        let Some(path) = &page.path else {
            return rows;
        };
        let Some(preset) = &page.preset else {
            rows.push(
                ui::label("Reading the preset…", text::CONTROL, theme::ink3()).into_any_element(),
            );
            return rows;
        };
        if let Some(error) = preset["error"].as_str() {
            rows.push(
                ui::label(
                    format!("{} could not be read: {error}", folder_of(path)),
                    text::CONTROL,
                    theme::neg(),
                )
                .into_any_element(),
            );
            return rows;
        }
        let made = preset["made"].as_array().cloned().unwrap_or_default();
        let mut title = format!(
            "{} · {} media slot{}",
            page.name().unwrap_or_default(),
            page.slots.len(),
            if page.slots.len() == 1 { "" } else { "s" }
        );
        if !made.is_empty() {
            title.push_str(&format!(" · makes {}", made.len()));
        }
        rows.push(
            ui::label(title, text::TITLE, theme::ink())
                .mt(px(theme::space::S4))
                .into_any_element(),
        );
        rows.extend(page.slots.iter().enumerate().map(|(index, slot)| {
            let read_by = presets::read_by_line(&presets::slot_read_by(preset, &slot.slot));
            let (status, ink): (SharedString, _) = match (&slot.file, &slot.fit) {
                (None, _) => ("No file chosen".into(), theme::ink3()),
                (Some(_), None) => ("Checking…".into(), theme::ink3()),
                (Some(_), Some(fit)) => (
                    fit.words.clone().into(),
                    if fit.fits { theme::pos() } else { theme::neg() },
                ),
            };
            let start_open = slot.fit.as_ref().is_some_and(Fit::start_open);
            let start = if start_open {
                field(&slot.start, window, cx)
                    .w(px(START_FIELD))
                    .into_any_element()
            } else {
                div()
                    .w(px(START_FIELD))
                    .h(px(height::H28))
                    .flex()
                    .items_center()
                    .px(px(theme::space::S8))
                    .rounded(px(radius::CONTROL))
                    .border_1()
                    .border_color(theme::line())
                    .child(ui::label("0", text::BODY, theme::dis()))
                    .into_any_element()
            };
            let file_tip = slot
                .file
                .as_ref()
                .map(|file| format!("{status}\n{}", file.display()));
            div()
                .id(ElementId::named_usize("launcher-slot", index))
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .py(px(theme::space::S6))
                .border_t_1()
                .border_color(theme::line())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(ui::line(slot.id.clone(), text::BODY_NAME, theme::ink()))
                        .child(ui::line(
                            presets::needs_line(&slot.slot),
                            text::META,
                            theme::ink3(),
                        ))
                        .when(!read_by.is_empty(), |column| {
                            column.child(ui::line(read_by, text::META, theme::ink3()))
                        }),
                )
                .child(
                    div()
                        .id(ElementId::named_usize("launcher-slot-status", index))
                        .w(px(150.0))
                        .flex_none()
                        .relative()
                        .child(ui::line(status, text::CONTROL, ink))
                        .child(ui::tip_zone(
                            ElementId::named_usize("launcher-slot-status-tip", index),
                            file_tip.map(ui::Tip::new),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(theme::space::S6))
                        .child(ui::label("Start", text::CONTROL, theme::ink2()))
                        .child(start)
                        .when_some(
                            slot.fit.as_ref().filter(|fit| fit.start_open()),
                            |row, fit| {
                                row.child(ui::label(
                                    format!("s · up to {:.2}", fit.spare),
                                    text::META,
                                    theme::ink3(),
                                ))
                            },
                        ),
                )
                .child(
                    self.ring(
                        Stop::SlotFile(index),
                        window,
                        ui::button(("launcher-slot-file", index), "Choose File…")
                            .small()
                            .disabled(!idle)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_slot_file(index, window, cx)
                            })),
                    ),
                )
                .into_any_element()
        }));
        if !made.is_empty() {
            rows.push(
                ui::label(
                    format!("Made by the preset · {} media", made.len()),
                    text::META_STRONG,
                    theme::ink3(),
                )
                .mt(px(theme::space::S8))
                .into_any_element(),
            );
            rows.extend(made.iter().enumerate().map(|(index, entry)| {
                let steps: Vec<&str> = entry["steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let steps = steps.join("\n");
                div()
                    .id(ElementId::named_usize("launcher-made", index))
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S12))
                    .py(px(theme::space::S6))
                    .border_t_1()
                    .border_color(theme::line())
                    .relative()
                    .child(ui::tip_zone(
                        ElementId::named_usize("launcher-made-tip", index),
                        Some(ui::Tip::new(steps)),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(ui::line(
                                entry["id"].as_str().unwrap_or_default().to_owned(),
                                text::BODY_NAME,
                                theme::ink(),
                            ))
                            .child(ui::line(
                                presets::made_line(entry),
                                text::META,
                                theme::ink3(),
                            )),
                    )
                    .child(ui::label("No file needed", text::CONTROL, theme::ink3()))
                    .into_any_element()
            }));
            rows.push(
                ui::label(
                    "Made after applying, with Make… in the bin.",
                    text::META,
                    theme::ink3(),
                )
                .into_any_element(),
            );
        }
        if !page.texts.is_empty() {
            let count = page.texts.len();
            rows.push(
                ui::label(
                    format!("Text · {count} field{}", if count == 1 { "" } else { "s" }),
                    text::META_STRONG,
                    theme::ink3(),
                )
                .mt(px(theme::space::S8))
                .into_any_element(),
            );
            rows.extend(page.texts.iter().enumerate().map(|(index, text_slot)| {
                ui::tip_on(
                    div()
                        .id(ElementId::named_usize("launcher-text", index))
                        .child(form_row_owned(
                            format!("Text {}", index + 1),
                            field(&text_slot.input, window, cx).flex_1(),
                        )),
                    Some(ui::Tip::new(text_slot.clip.clone())),
                )
                .into_any_element()
            }));
        }
        rows.push(
            ui::hairline(true)
                .mt(px(theme::space::S4))
                .into_any_element(),
        );
        rows
    }

    fn form(&mut self, mode: Mode, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let idle = self.busy.is_none();
        let problem = if idle { self.form_problem(cx) } else { None };
        let location = self.location(cx);
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let (destination, act_id, act_words): (String, &'static str, &'static str) = match mode {
            Mode::New | Mode::Preset => {
                if mode == Mode::Preset {
                    rows.extend(self.preset_rows(window, cx));
                }
                let folder = folder_name(self.name.read(cx).text());
                rows.push(form_row("Name", field(&self.name, window, cx).flex_1()));
                rows.push(form_row("Location", self.location_row(window, cx)));
                if mode == Mode::New {
                    let formats = Format::ALL.map(|format| (format, format.label()));
                    rows.push(form_row(
                        "Format",
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme::space::S12))
                            .child(self.ring(
                                Stop::Format,
                                window,
                                ui::segmented(
                                    "launcher-format",
                                    &formats,
                                    self.format,
                                    Self::chooser(cx, |this, format, _, cx| {
                                        this.format = format;
                                        cx.notify();
                                    }),
                                ),
                            ))
                            .child(ui::label(self.format.size(), text::CONTROL, theme::ink3())),
                    ));
                }
                rows.extend(self.details_rows(window, cx));
                let destination = if folder.is_empty() {
                    String::new()
                } else {
                    format!("Creates {}", home_relative(&location.join(folder)))
                };
                (destination, "launcher-create", "Create Project")
            }
            Mode::Clone => {
                rows.push(form_row(
                    "Repository",
                    field(&self.url, window, cx).flex_1(),
                ));
                rows.push(form_row("Location", self.location_row(window, cx)));
                rows.push(form_row_owned(
                    String::new(),
                    ui::label(
                        "Kerf configures the repository so project.kerf merges by clip and track, and downloads Git LFS media when it is installed.",
                        text::CONTROL,
                        theme::ink3(),
                    )
                    .flex_1()
                    .min_w_0(),
                ));
                let destination = repository_name(self.url.read(cx).text())
                    .map(|folder| format!("Clones into {}", home_relative(&location.join(folder))))
                    .unwrap_or_default();
                (destination, "launcher-clone", "Clone")
            }
        };
        // Why it cannot be made yet, as it is typed; else where it goes.
        let (foot, foot_ink) = match &problem {
            Some(problem) => (problem.clone(), theme::warn()),
            None => (destination, theme::ink3()),
        };
        // Greyed by what is typed, its tip says why.
        let act_button = ui::button(act_id, act_words)
            .primary()
            .disabled(!idle || problem.is_some())
            .on_click(cx.listener(|this, _, window, cx| this.choose(window, cx)));
        let act_button = match problem.clone() {
            Some(why) => act_button.reason(why),
            None => act_button,
        };
        let act = div()
            .id("launcher-act-why")
            .child(self.ring(Stop::Act, window, act_button));
        div()
            .mt(px(theme::space::S12))
            .flex()
            .flex_col()
            .gap(px(theme::space::S12))
            .p(px(theme::space::S16))
            .rounded(px(radius::CARD))
            .bg(theme::chrome())
            .border_1()
            .border_color(theme::line())
            .children(rows)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S12))
                    .pt(px(theme::space::S4))
                    .child(ui::line(foot, text::CONTROL, foot_ink).flex_1().min_w_0())
                    .child(act),
            )
            .with_animation(
                ElementId::Name(format!("launcher-form-{mode:?}").into()),
                motion::animation(theme::motion::APPEAR),
                |form, t| form.opacity(t),
            )
    }

    fn status(&self) -> Option<impl IntoElement> {
        if let Some(busy) = &self.busy {
            let dot = ui::dot(theme::activity::gpu(), 6.0);
            return Some(
                div()
                    .mt(px(theme::space::S12))
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(dot)
                    .child(
                        ui::label(busy.clone(), text::CONTROL, theme::ink2())
                            .flex_1()
                            .min_w_0(),
                    ),
            );
        }
        self.problem.as_ref().map(|problem| {
            div()
                .mt(px(theme::space::S12))
                .flex()
                .items_center()
                .gap(px(theme::space::S8))
                .child(ui::dot(theme::neg(), 6.0))
                .child(
                    ui::label(problem.clone(), text::CONTROL, theme::neg())
                        .flex_1()
                        .min_w_0(),
                )
        })
    }

    fn recent_list(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let idle = self.busy.is_none();
        let keyed = self
            .handle(Stop::Recent)
            .is_some_and(|handle| handle.is_focused(window));
        let rows = self.recent.iter().enumerate().map(|(index, recent)| {
            let ink = if recent.exists {
                theme::ink()
            } else {
                theme::dis()
            };
            let meta = recent.meta();
            let path = recent.path.clone();
            let open = recent.exists && idle;
            let chosen = keyed && index == self.recent_chosen;
            div()
                .id(ElementId::named_usize("launcher-recent", index))
                .h(px(RECENT_ROW))
                .px(px(theme::space::S12))
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .rounded(px(radius::CARD))
                .border_1()
                .border_color(if chosen {
                    theme::accent()
                } else {
                    theme::with_alpha(theme::accent(), 0.0)
                })
                .when(chosen, |row| row.bg(theme::chrome()))
                .relative()
                .child(ui::tip_zone(
                    ElementId::named_usize("launcher-recent-tip", index),
                    Some(ui::Tip::new(recent.path.display().to_string())),
                ))
                .when(open, |row| {
                    row.cursor_pointer()
                        .hover(|style| style.bg(theme::chrome()))
                })
                .child(Icon::Folder.view(
                    16.0,
                    if recent.exists {
                        theme::ink2()
                    } else {
                        theme::dis()
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(ui::line(recent.name.clone(), text::BODY_NAME, ink))
                        .child(ui::line(
                            recent.place.clone(),
                            text::CONTROL,
                            if recent.exists {
                                theme::ink3()
                            } else {
                                theme::dis()
                            },
                        )),
                )
                .when_some(recent.branch.clone(), |row, branch| {
                    row.child(ui::chip(branch, theme::ink2()))
                })
                .child(ui::line(meta, text::META, theme::ink3()).flex_none())
                .when(!recent.exists, |row| {
                    let forget = path.clone();
                    row.child(
                        icon_button(
                            ElementId::named_usize("launcher-forget", index),
                            Icon::Close,
                        )
                        .size(height::H24)
                        .tooltip("Remove from Recent")
                        .on_click(cx.listener(move |this, _, _, cx| this.forget(&forget, cx))),
                    )
                })
                .when(open, |row| {
                    row.on_click(cx.listener(move |this, _, window, cx| {
                        this.recent_chosen = index;
                        this.open_chosen(window, cx)
                    }))
                })
                .with_animation(
                    ElementId::named_usize("launcher-recent-in", index),
                    staggered(index),
                    |row, t| {
                        row.opacity(t)
                            .relative()
                            .top(px(theme::motion::APPEAR_RISE * (1.0 - t)))
                    },
                )
        });
        let mut list = div()
            .id("launcher-recent-list")
            .key_context(CONTROL)
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .children(rows);
        if let Some(handle) = self.handle(Stop::Recent) {
            list = list.track_focus(&handle);
        }
        div()
            .mt(px(40.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(theme::space::S12))
                    .pb(px(theme::space::S6))
                    .child(ui::label("Recent", text::META_STRONG, theme::ink3()).flex_1())
                    .when(!self.recent.is_empty(), |header| {
                        header.child(
                            self.ring(
                                Stop::ClearRecent,
                                window,
                                ui::button("launcher-clear-recent", "Clear")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.clear_recent(window, cx)
                                    })),
                            ),
                        )
                    }),
            )
            .when(self.recent.is_empty(), |list| {
                list.child(
                    ui::label(
                        "No recent projects yet. Projects you open appear here.",
                        text::CONTROL,
                        theme::ink3(),
                    )
                    .px(px(theme::space::S12)),
                )
            })
            .child(list)
    }

    /// A segmented control's or a switch's choice, carried to `this`.
    fn chooser<T, F>(cx: &Context<Self>, choose: F) -> impl Fn(T, &mut Window, &mut App) + 'static
    where
        T: Copy + 'static,
        F: Fn(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    {
        let this = cx.entity().downgrade();
        move |value, window, cx| {
            let _ = this.update(cx, |this, cx| choose(this, value, window, cx));
        }
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.arrange(cx);
        let mode = self.mode;
        let settings = self.settings_over(window);
        let form = mode.map(|mode| self.form(mode, window, cx));
        let column = div()
            .id("launcher")
            .size_full()
            .overflow_y_scroll()
            .bg(theme::surround())
            .font(theme::ui_font(FontWeight::NORMAL))
            .text_color(theme::ink())
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(COLUMN + 2.0 * theme::space::S16))
                    .px(px(theme::space::S16))
                    .pt(px(TOP))
                    .pb(px(48.0))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .child(img(app_icon()).size(px(56.0)).flex_none())
                            .child(div().flex_1())
                            .child(self.ring(
                                Stop::Settings,
                                window,
                                ui::button("launcher-settings", "Settings…")
                                    .ghost()
                                    .icon(Icon::Gear)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.show_settings(Page::Chat, window, cx)
                                    })),
                            )),
                    )
                    .child(ui::label("What are we making?", text::DISPLAY, theme::ink()).mt(px(20.0)))
                    .child(
                        ui::label(
                            "A project is a folder: its project.kerf, its media and its presets. Kerf keeps it as you work.",
                            text::BODY,
                            theme::ink3(),
                        )
                        .mt(px(theme::space::S4)),
                    )
                    .child(
                        div()
                            .mt(px(28.0))
                            .flex()
                            .gap(px(theme::space::S12))
                            .child(self.way(Some(Mode::New), Icon::Plus, "New Project", "An empty project, in a folder of its own", window, cx))
                            .child(self.way(Some(Mode::Preset), Icon::Sparkle, "From Preset", "A shared preset, filled with your media", window, cx))
                            .child(self.way(None, Icon::Folder, "Open Folder…", "A project, or a folder of footage to start one from", window, cx))
                            .child(self.way(Some(Mode::Clone), Icon::Import, "Clone from GitHub", "A shared project, from its repository", window, cx)),
                    )
                    .children(form)
                    .children(self.status())
                    .child(self.recent_list(window, cx))
                    // It comes up into place once, as the launcher shows.
                    .with_animation("launcher-rise", motion::animation(theme::motion::RISE), |column, t| {
                        column.opacity(t).relative().top(px(theme::motion::PAGE_RISE * (1.0 - t)))
                    }),
            );
        // The window's commands answer here too: ⌘, opens Settings. The
        // launcher's own keys answer only under it, not in Settings.
        div()
            .key_context(WORKSPACE)
            .on_action(cx.listener(Self::command))
            .relative()
            .size_full()
            .child(
                div()
                    .key_context(CONTEXT)
                    .track_focus(&self.focus_handle)
                    .on_action(cx.listener(|this, _: &NextControl, window, cx| {
                        this.step_focus(1, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &PreviousControl, window, cx| {
                        this.step_focus(-1, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &Choose, window, cx| this.choose(window, cx)))
                    .on_action(cx.listener(|this, _: &Back, window, cx| this.back(window, cx)))
                    .on_action(cx.listener(|this, _: &ChosenUp, window, cx| {
                        this.step_chosen(-1, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &ChosenDown, window, cx| {
                        this.step_chosen(1, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &Press, window, cx| this.press(window, cx)))
                    .on_action(
                        cx.listener(|this, _: &Earlier, window, cx| this.arrow(-1, window, cx)),
                    )
                    .on_action(cx.listener(|this, _: &Later, window, cx| this.arrow(1, window, cx)))
                    .size_full()
                    .child(column),
            )
            .children(settings)
    }
}

// ---------------------------------------------------------------- pieces

/// A form's row: what it asks, and the control that answers.
fn form_row(label: &'static str, control: impl IntoElement) -> gpui::AnyElement {
    form_row_owned(label.to_owned(), control)
}

fn form_row_owned(label: String, control: impl IntoElement) -> gpui::AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme::space::S12))
        .child(
            ui::label(label, text::CONTROL, theme::ink2())
                .w(px(FORM_LABEL))
                .flex_none(),
        )
        .child(div().flex_1().min_w_0().flex().child(control))
        .into_any_element()
}

/// A text field in its frame, ringed in the accent while typed in. A press
/// anywhere in the frame types in it. The caller gives it its width.
pub(crate) fn field(input: &Entity<TextInput>, window: &Window, cx: &App) -> gpui::Div {
    let handle = input.read(cx).focus_handle(cx);
    let focused = handle.is_focused(window);
    div()
        .h(px(height::H28))
        .flex()
        .items_center()
        .px(px(theme::space::S8))
        .overflow_hidden()
        .rounded(px(radius::CONTROL))
        .bg(theme::well())
        .border_1()
        .border_color(if focused {
            theme::accent()
        } else {
            theme::line2()
        })
        .on_mouse_down(MouseButton::Left, move |_, window, _| {
            window.focus(&handle);
            window.prevent_default();
        })
        .child(input.clone())
}

/// A row of a list coming in `index` places after the first: it waits
/// [`theme::motion::POP_STAGGER`] for each, then fades up over `pop`.
fn staggered(index: usize) -> gpui::Animation {
    let delay = theme::motion::POP_STAGGER * index.min(12) as u32;
    let whole = delay + theme::motion::POP;
    let start = delay.as_secs_f32() / whole.as_secs_f32();
    gpui::Animation::new(motion::duration(whole).max(Duration::from_millis(1))).with_easing(
        move |t| {
            if motion::reduced() || t >= 1.0 {
                1.0
            } else if t <= start {
                0.0
            } else {
                motion::ease((t - start) / (1.0 - start))
            }
        },
    )
}

/// Kerf's icon, in colour.
pub(crate) fn app_icon() -> Arc<gpui::Image> {
    static ICON: OnceLock<Arc<gpui::Image>> = OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            include_bytes!("../assets/icon/industrial-kerf-128.png").to_vec(),
        ))
    })
    .clone()
}

/// `path` as a person writes it: `~/Videos/Kerf` under the home folder.
pub(crate) fn home_relative(path: &Path) -> String {
    if let Some(rest) =
        prefs::home().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf))
    {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
    }
    path.display().to_string()
}

/// A path as typed, with `~` for the home folder.
fn expanded(text: &str) -> PathBuf {
    let home = prefs::home();
    match (text.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home,
        (Some(rest), Some(home)) if rest.starts_with(['/', '\\']) => home.join(&rest[1..]),
        _ => PathBuf::from(text),
    }
}

/// How long ago `time` was, as a person says it.
pub(crate) fn ago(time: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(time)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let plural = |count: u64, unit: &str| {
        if count == 1 {
            format!("1 {unit} ago")
        } else {
            format!("{count} {unit}s ago")
        }
    };
    match seconds {
        0..60 => "just now".into(),
        60..3_600 => plural(seconds / 60, "minute"),
        3_600..86_400 => plural(seconds / 3_600, "hour"),
        86_400..172_800 => "yesterday".into(),
        172_800..1_209_600 => plural(seconds / 86_400, "day"),
        1_209_600..5_184_000 => plural(seconds / 604_800, "week"),
        5_184_000..63_072_000 => plural(seconds / 2_592_000, "month"),
        _ => plural(seconds / 31_536_000, "year"),
    }
}

/// The Kerf folder among the person's videos.
fn default_videos() -> PathBuf {
    let home = prefs::home().unwrap_or_else(|| PathBuf::from("."));
    let videos = if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    };
    home.join(videos).join("Kerf")
}

/// Where new projects go: beside the last one opened, else a Kerf folder
/// among the person's videos.
fn default_location(prefs: &Prefs) -> PathBuf {
    if let Some(folder) = prefs
        .recent
        .iter()
        .filter_map(|project| project.parent())
        .find(|folder| folder.is_dir())
    {
        return folder.to_path_buf();
    }
    default_videos()
}

/// The first of Untitled, Untitled 2, … free in `location`.
fn untitled(location: &Path) -> String {
    let mut name = "Untitled".to_owned();
    for n in 2..1000 {
        if !occupied(&location.join(&name)) {
            break;
        }
        name = format!("Untitled {n}");
    }
    name
}

/// A folder a new project or a clone cannot go into: a file, or a folder
/// with anything in it.
fn occupied(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    !path.is_dir()
        || std::fs::read_dir(path)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
}

/// The folder a project named `name` gets, as kerf-collab names it: the
/// name without the characters file systems refuse.
fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '-'
            } else {
                c
            }
        })
        .collect();
    cleaned.trim_matches('.').trim().to_owned()
}

fn folder_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// When its document last changed.
fn edited(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path.join("project.kerf"))
        .and_then(|metadata| metadata.modified())
        .ok()
}

/// Whether `url` reads as an address Git clones from.
pub fn git_address(url: &str) -> bool {
    let url = url.trim();
    if ["https://", "http://", "ssh://", "git://", "file://"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
    {
        return url
            .split("://")
            .nth(1)
            .is_some_and(|rest| rest.contains('/'));
    }
    // user@host:path, as SSH addresses are written.
    match (url.find('@'), url.find(':')) {
        (Some(at), Some(colon)) => {
            at > 0 && colon > at + 1 && colon + 1 < url.len() && !url.contains(' ')
        }
        _ => false,
    }
}

/// The folder a clone of `url` gets: its last part without `.git`, as Git
/// names it.
fn repository_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty() && name != trimmed).then(|| name.to_owned())
}
