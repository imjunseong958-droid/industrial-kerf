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
//! puts the form away. Opened from an editor (New Project…), it can be
//! given up: its Cancel, or Esc with no form open, closes it and the
//! editor comes back.

mod details;
mod form;
mod keys;
mod plan;
mod preset;
mod preset_rows;
mod recent;
mod tasks;

pub use details::{github_account, tools_hint};
pub use form::field;
pub use keys::{
    launcher_escape, Back, Choose, ChosenDown, ChosenUp, Earlier, Later, LauncherEscape,
    NextControl, Press, PreviousControl,
};
pub(crate) use plan::home_relative;
pub use plan::{clone_problem, details_problem, git_address, Plan};
pub(crate) use recent::ago;

use crate::presets::{self, Picked};
use crate::settings::{Page, SettingsEvent, SettingsView};
use details::RATES;
use editor_session::commands::{RunCommand, WORKSPACE};
use editor_session::session::Session;
use editor_state::prefs::Prefs;
use editor_ui::icons::Icon;
use editor_ui::input::{InputEvent, TextInput};
use editor_ui::motion;
use editor_ui::theme::{self, radius, text};
use editor_ui::ui;
use form::Format;
use gpui::{
    div, img, prelude::*, px, AnimationExt, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, MouseButton, PathPromptOptions, PromptButton, PromptLevel, SharedString,
    Subscription, Task, Window,
};
use keys::{bind_keys, Stop};
use plan::{default_location, folder_of, untitled};
use preset::PresetPage;
use recent::Recent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tasks::TASK_POLL;

pub enum LauncherEvent {
    /// Open `project`; `starts` when the folder was no project yet, so its
    /// media comes in with it. `warnings` are what making or cloning it
    /// warned of, said in its window once it is open: what opens it hands
    /// them to `kerf_editor::app::warn_when_open` first.
    Open {
        project: PathBuf,
        starts: bool,
        warnings: Vec<String>,
    },
    /// Given up (its Cancel, or Esc with no form open), when it was opened
    /// from an editor ([`LauncherView::over_editor`]): its window goes and
    /// the editor's comes back.
    Cancel,
}

/// The launcher's column, and the air above it.
const COLUMN: f32 = 640.0;
const TOP: f32 = 88.0;

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
    /// Opened from an editor (New Project…): it can be given up, by its
    /// Cancel or Esc, and the editor comes back.
    over_editor: bool,
    /// The controls the keyboard goes to, each with its own focus for as
    /// long as the launcher is (a preset's slots, from when they come);
    /// the order Tab goes in is worked out from what shows.
    stops: HashMap<Stop, FocusHandle>,
    /// The controls on screen in the order Tab goes, each with its focus,
    /// as last drawn ([`Self::arrange`]).
    order: Vec<(Stop, FocusHandle)>,
    _task: Option<Task<()>>,
    _tools: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LauncherEvent> for LauncherView {}

impl LauncherView {
    /// The launcher, as it was left (its way, template, frame rate and
    /// Details, from `prefs`), with the kerfd the editor was started with
    /// ([`editor_session::session::started_kerfd`]) unless [`Self::with_kerfd`] says
    /// another.
    pub fn new(prefs: Prefs, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let kerfd = editor_session::session::started_kerfd(cx);
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
        // Each control the keys go to has its focus from the start; a
        // preset's slots have theirs as they come.
        let stops = Stop::FIXED
            .into_iter()
            .map(|stop| (stop, cx.focus_handle()))
            .collect();
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
            over_editor: false,
            stops,
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

    /// Opened from an editor (New Project…): it has Cancel, and Esc with
    /// no form open gives it up too
    /// ([`LauncherEvent::Cancel`]). The launcher the editor starts with
    /// has neither.
    pub fn over_editor(mut self) -> Self {
        self.over_editor = true;
        self
    }

    /// Gives the launcher up, back to the editor behind it.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.over_editor {
            cx.emit(LauncherEvent::Cancel);
        }
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
                .bg(theme::with_alpha(theme::scrim(), theme::motion::SCRIM))
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

    /// Esc: the form goes, and the keyboard is on the launcher again; with
    /// none open, a launcher opened from an editor gives it back
    /// ([`launcher_escape`]).
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match launcher_escape(self.mode.is_some(), self.busy.is_some(), self.over_editor) {
            LauncherEscape::PutFormAway => {
                self.mode = None;
                self.problem = None;
            }
            LauncherEscape::GiveBack => return self.cancel(cx),
            LauncherEscape::Nothing => {}
        }
        window.focus(&self.focus_handle);
        cx.notify();
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
        let starts = starts.unwrap_or_else(|| editor_state::folders::needs_start(&project));
        cx.emit(LauncherEvent::Open {
            project,
            starts,
            warnings,
        });
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
    ) -> Option<impl std::future::Future<Output = Result<usize, impl Sized + use<>>> + use<>> {
        if !editor_state::folders::needs_start(&folder) {
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

    /// The launcher's head: Kerf's mark, Settings…, and Cancel when an editor
    /// is behind it.
    fn head(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .items_start()
            .child(img(app_icon()).size(px(56.0)).flex_none())
            .child(div().flex_1())
            .child(
                self.ring(
                    Stop::Settings,
                    window,
                    ui::button("launcher-settings", "Settings…")
                        .ghost()
                        .icon(Icon::Gear)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_settings(Page::Chat, window, cx)
                        })),
                ),
            )
            // From an editor it can be given up, and the
            // project open there comes back.
            .when(self.over_editor, |head| {
                head.child(
                    self.ring(
                        Stop::Cancel,
                        window,
                        ui::button("launcher-cancel", "Cancel")
                            .tooltip("Back to the project")
                            .key("Esc")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
                )
            })
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
                    .child(self.head(window, cx))
                    .child(ui::label("What are we making?", text::DISPLAY, theme::ink()).mt(px(20.0)))
                    .child(
                        ui::label(
                            "A project is a folder: its project.kerf, its media and its presets. Kerf keeps it as you work.",
                            text::BODY,
                            theme::ink3(),
                        )
                        .mt(px(theme::space::S4)),
                    )
                    .child(self.ways(window, cx))
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
            .child(self.with_keys(column, cx))
            .children(settings)
    }
}

// ---------------------------------------------------------------- pieces

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
            include_bytes!("../../../assets/icon/industrial-kerf-128.png").to_vec(),
        ))
    })
    .clone()
}
