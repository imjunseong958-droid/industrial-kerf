//! The project launcher: what opens with no project. Kerf's mark and one
//! question, the ways to begin (a new project, a folder, a clone from
//! GitHub), and the projects opened before. Making and cloning run as the
//! bridge's project tasks, on a thread of their own, polled until they end.
//! Settings opens over it as over a project's window, with no project: what
//! answers in the conversation, its key, how the editor looks, its keys.

use crate::commands::{RunCommand, WORKSPACE};
use crate::icons::Icon;
use crate::input::{InputEvent, TextInput};
use crate::motion;
use crate::prefs::{self, Prefs};
use crate::session::Session;
use crate::settings::{Page, SettingsEvent, SettingsView};
use crate::theme::{self, height, radius, text};
use crate::ui::{self, icon_button};
use gpui::{
    div, img, prelude::*, px, AnimationExt, App, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, MouseButton, PathPromptOptions, SharedString, Subscription,
    Task, Window,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

pub enum LauncherEvent {
    /// Open `project`; `starts` when the folder was no project yet, so its
    /// media comes in with it.
    Open { project: PathBuf, starts: bool },
}

/// How often a running project task is asked how it is getting on.
const TASK_POLL: Duration = Duration::from_millis(100);
/// The launcher's column, and the air above it.
const COLUMN: f32 = 640.0;
const TOP: f32 = 88.0;
/// A form's labels stand in a column this wide.
const FORM_LABEL: f32 = 96.0;
/// A recent project's row.
const RECENT_ROW: f32 = 52.0;

/// What is being begun, below the three ways to begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    New,
    Clone,
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
        }
    }

    /// Its document's name and its branch too, which read files and ask Git.
    fn inspected(path: &Path) -> Self {
        let mut recent = Self::quick(path);
        let info = kerf_bridge::project_inspect(&json!({"path": path.to_string_lossy()}));
        if let Some(name) = info["name"].as_str().filter(|name| !name.trim().is_empty()) {
            recent.name = name.to_owned();
        }
        recent.branch = info["branch"].as_str().map(str::to_owned);
        recent
    }
}

pub struct LauncherView {
    prefs: Prefs,
    focus_handle: FocusHandle,
    mode: Option<Mode>,
    name: Entity<TextInput>,
    location: Entity<TextInput>,
    url: Entity<TextInput>,
    format: Format,
    rate: &'static str,
    git: bool,
    recent: Vec<Recent>,
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
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LauncherEvent> for LauncherView {}

impl LauncherView {
    pub fn new(prefs: Prefs, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let mut subscriptions = Vec::new();
        for input in [&name, &location_input, &url] {
            subscriptions.push(cx.subscribe_in(input, window, Self::input_event));
            let handle = input.read(cx).focus_handle(cx);
            subscriptions.push(cx.on_focus(&handle, window, |_, _, cx| cx.notify()));
            subscriptions.push(cx.on_blur(&handle, window, |_, _, cx| cx.notify()));
        }

        // The recent projects show at once by their folders; their names
        // and branches follow, since finding them reads files and asks Git.
        let recent = prefs
            .recent
            .iter()
            .map(|path| Recent::quick(path))
            .collect();
        let paths = prefs.recent.clone();
        let inspecting = cx.background_executor().spawn(async move {
            paths
                .iter()
                .map(|path| Recent::inspected(path))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let inspected = inspecting.await;
            let _ = this.update(cx, |this, cx| {
                this.recent = inspected
                    .into_iter()
                    .filter(|recent| this.prefs.recent.contains(&recent.path))
                    .collect();
                cx.notify();
            });
        })
        .detach();

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Self {
            prefs,
            focus_handle,
            mode: None,
            name,
            location: location_input,
            url,
            format: Format::Standard,
            rate: "30",
            git: false,
            recent,
            busy: None,
            problem: None,
            settings: None,
            settings_opened: 0,
            session: None,
            _task: None,
            _subscriptions: subscriptions,
        }
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
                let session = Session::open(None, None, self.prefs.clone(), window, cx);
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
        // What Settings saved, for the projects listed here.
        self.prefs.refresh();
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
    fn settings_over(&self) -> Option<gpui::AnyElement> {
        let (settings, opened) = self.settings.as_ref()?;
        Some(
            div()
                .id("launcher-scrim")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::with_alpha(gpui::rgb(0x000000), theme::motion::SCRIM))
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .relative()
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
            InputEvent::Submit => match self.mode {
                Some(Mode::New) => self.create(cx),
                Some(Mode::Clone) => self.clone_repository(cx),
                None => {}
            },
            InputEvent::Cancel => {
                if self.busy.is_none() {
                    self.mode = None;
                    window.focus(&self.focus_handle);
                    cx.notify();
                }
            }
        }
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
            }
        }
        cx.notify();
    }

    /// Opens `project`; `starts` unless said is whether it is no project yet.
    fn open(&mut self, project: PathBuf, starts: Option<bool>, cx: &mut Context<Self>) {
        let starts = starts.unwrap_or_else(|| !project.join("project.kerf").is_file());
        cx.emit(LauncherEvent::Open { project, starts });
    }

    fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let chosen = chosen.await;
            let _ = this.update(cx, |this, cx| match chosen {
                Ok(Ok(Some(paths))) => {
                    if let Some(project) = paths.into_iter().next() {
                        this.open(project, None, cx);
                    }
                }
                Ok(Err(error)) => {
                    this.problem = Some(format!("No folder chooser could open: {error}").into());
                    cx.notify();
                }
                Ok(Ok(None)) | Err(_) => {}
            });
        })
        .detach();
    }

    fn choose_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let chosen = chosen.await;
            let _ = this.update(cx, |this, cx| match chosen {
                Ok(Ok(Some(paths))) => {
                    if let Some(folder) = paths.into_iter().next() {
                        this.location
                            .update(cx, |input, cx| input.set_text(home_relative(&folder), cx));
                    }
                }
                Ok(Err(error)) => {
                    this.problem = Some(
                        format!("No folder chooser could open: {error}; type the folder instead")
                            .into(),
                    );
                    cx.notify();
                }
                Ok(Ok(None)) | Err(_) => {}
            });
        })
        .detach();
    }

    /// Where the project or the clone goes, as typed.
    fn location(&self, cx: &App) -> PathBuf {
        expanded(self.location.read(cx).text().trim())
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let name = self.name.read(cx).text().trim().to_owned();
        let location = self.location(cx);
        let folder = folder_name(&name);
        let problem = if folder.is_empty() {
            Some("Name the project".to_owned())
        } else if location.as_os_str().is_empty() {
            Some("Choose where to save the project".to_owned())
        } else if !location.is_absolute() {
            Some("Choose a full location, such as ~/Videos/Kerf".to_owned())
        } else if occupied(&location.join(&folder)) {
            Some(format!(
                "{} already exists",
                home_relative(&location.join(&folder))
            ))
        } else {
            None
        };
        if let Some(problem) = problem {
            self.problem = Some(problem.into());
            cx.notify();
            return;
        }
        let request = json!({
            "type": "create",
            "name": name,
            "location": location,
            "template": self.format.wire(),
            "rate": self.rate,
            "git": self.git,
        });
        self.run(request, format!("Making {folder}…"), true, cx);
    }

    fn clone_repository(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let url = self.url.read(cx).text().trim().to_owned();
        let location = self.location(cx);
        let folder = repository_name(&url);
        let problem = if url.is_empty() {
            Some("Enter the repository's address".to_owned())
        } else if !git_address(&url) || folder.is_none() {
            Some("Enter a Git address such as https://github.com/you/film.git".to_owned())
        } else if location.as_os_str().is_empty() || !location.is_absolute() {
            Some("Choose a full location to put the copy in, such as ~/Videos/Kerf".to_owned())
        } else if folder
            .as_ref()
            .is_some_and(|folder| occupied(&location.join(folder)))
        {
            Some(format!(
                "{} already exists",
                home_relative(&location.join(folder.as_deref().unwrap_or_default()))
            ))
        } else {
            None
        };
        if let Some(problem) = problem {
            self.problem = Some(problem.into());
            cx.notify();
            return;
        }
        let request = json!({"type": "clone", "url": url, "location": location});
        self.run(
            request,
            format!("Cloning {}…", folder.unwrap_or_default()),
            false,
            cx,
        );
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
                for warning in status["warnings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    log::warn!("{warning}");
                }
                match status["path"].as_str().filter(|path| !path.is_empty()) {
                    Some(path) => self.open(PathBuf::from(path), made.then_some(true), cx),
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

    /// Takes a project that is no longer there out of the recent ones.
    fn forget(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.prefs.recent.retain(|known| known != path);
        self.prefs.save();
        self.recent.retain(|recent| recent.path != path);
        cx.notify();
    }

    // ------------------------------------------------------------ drawing

    fn way(
        &self,
        mode: Option<Mode>,
        icon: Icon,
        title: &'static str,
        hint: &'static str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let on = mode.is_some() && mode == self.mode;
        let idle = self.busy.is_none();
        div()
            .id(title)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(theme::space::S6))
            .p(px(theme::space::S16))
            .rounded(px(radius::CARD))
            .bg(if on { theme::card() } else { theme::chrome() })
            .border_1()
            .border_color(if on { theme::accent() } else { theme::line() })
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
            }))
    }

    fn form(&self, mode: Mode, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let idle = self.busy.is_none();
        let location = self.location(cx);
        let location_row = div()
            .flex_1()
            .min_w_0()
            .flex()
            .gap(px(theme::space::S8))
            .child(field(&self.location, window, cx).flex_1())
            .child(
                ui::button("launcher-location", "Choose…")
                    .disabled(!idle)
                    .on_click(cx.listener(|this, _, window, cx| this.choose_location(window, cx))),
            );
        let (rows, destination, act): (Vec<gpui::AnyElement>, String, gpui::AnyElement) = match mode
        {
            Mode::New => {
                let formats = Format::ALL.map(|format| (format, format.label()));
                let folder = folder_name(self.name.read(cx).text());
                let destination = if folder.is_empty() {
                    String::new()
                } else {
                    format!("Creates {}", home_relative(&location.join(folder)))
                };
                (
                    vec![
                        form_row("Name", field(&self.name, window, cx).flex_1()),
                        form_row("Location", location_row),
                        form_row(
                            "Format",
                            div()
                                .flex()
                                .items_center()
                                .gap(px(theme::space::S12))
                                .child(ui::segmented("launcher-format", &formats, self.format, Self::chooser(cx, |this, format, _, cx| {
                                    this.format = format;
                                    cx.notify();
                                })))
                                .child(ui::label(self.format.size(), text::CONTROL, theme::ink3())),
                        ),
                        form_row(
                            "Frame rate",
                            div()
                                .flex()
                                .items_center()
                                .gap(px(theme::space::S12))
                                .child(ui::segmented("launcher-rate", &RATES, self.rate, Self::chooser(cx, |this, rate, _, cx| {
                                    this.rate = rate;
                                    cx.notify();
                                })))
                                .child(ui::label("fps", text::CONTROL, theme::ink3())),
                        ),
                        form_row(
                            "Share",
                            div()
                                .flex()
                                .items_center()
                                .gap(px(theme::space::S12))
                                .child(ui::switch("launcher-git", self.git, Self::chooser(cx, |this, on, _, cx| {
                                    this.git = on;
                                    cx.notify();
                                })))
                                .child(ui::label(
                                    "Make it a Git repository, so collaborators pull, merge and push its edits",
                                    text::CONTROL,
                                    theme::ink3(),
                                )),
                        ),
                    ],
                    destination,
                    ui::button("launcher-create", "Create Project")
                        .primary()
                        .disabled(!idle)
                        .on_click(cx.listener(|this, _, _, cx| this.create(cx)))
                        .into_any_element(),
                )
            }
            Mode::Clone => {
                let destination = repository_name(self.url.read(cx).text())
                    .map(|folder| format!("Clones into {}", home_relative(&location.join(folder))))
                    .unwrap_or_default();
                (
                    vec![
                        form_row("Repository", field(&self.url, window, cx).flex_1()),
                        form_row("Location", location_row),
                    ],
                    destination,
                    ui::button("launcher-clone", "Clone")
                        .primary()
                        .disabled(!idle)
                        .on_click(cx.listener(|this, _, _, cx| this.clone_repository(cx)))
                        .into_any_element(),
                )
            }
        };
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
                    .child(
                        ui::line(destination, text::CONTROL, theme::ink3())
                            .flex_1()
                            .min_w_0(),
                    )
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

    fn recent_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let idle = self.busy.is_none();
        let rows = self.recent.iter().enumerate().map(|(index, recent)| {
            let ink = if recent.exists {
                theme::ink()
            } else {
                theme::dis()
            };
            let meta: SharedString = if !recent.exists {
                "Not found".into()
            } else if let Some(edited) = recent.edited {
                format!("Edited {}", ago(edited)).into()
            } else {
                "No project yet".into()
            };
            let path = recent.path.clone();
            let open = recent.exists && idle;
            div()
                .id(ElementId::named_usize("launcher-recent", index))
                .h(px(RECENT_ROW))
                .px(px(theme::space::S12))
                .flex()
                .items_center()
                .gap(px(theme::space::S12))
                .rounded(px(radius::CARD))
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
                    row.on_click(
                        cx.listener(move |this, _, _, cx| this.open(path.clone(), Some(false), cx)),
                    )
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
        div()
            .mt(px(40.0))
            .flex()
            .flex_col()
            .gap(px(theme::space::S2))
            .child(
                ui::label("Recent", text::META_STRONG, theme::ink3())
                    .px(px(theme::space::S12))
                    .pb(px(theme::space::S6)),
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
            .children(rows)
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
        let mode = self.mode;
        let settings = self.settings_over();
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
                            .child(
                                ui::button("launcher-settings", "Settings…")
                                    .ghost()
                                    .icon(Icon::Gear)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.show_settings(Page::Chat, window, cx)
                                    })),
                            ),
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
                            .child(self.way(Some(Mode::New), Icon::Plus, "New Project", "An empty project, in a folder of its own", cx))
                            .child(self.way(None, Icon::Folder, "Open Folder…", "A project, or a folder of footage to start one from", cx))
                            .child(self.way(Some(Mode::Clone), Icon::Import, "Clone from GitHub", "A shared project, from its repository", cx)),
                    )
                    .when_some(mode, |column, mode| column.child(self.form(mode, window, cx)))
                    .children(self.status())
                    .child(self.recent_list(cx))
                    // It comes up into place once, as the launcher shows.
                    .with_animation("launcher-rise", motion::animation(theme::motion::RISE), |column, t| {
                        column.opacity(t).relative().top(px(theme::motion::PAGE_RISE * (1.0 - t)))
                    }),
            );
        // The window's commands answer here too: ⌘, opens Settings.
        div()
            .key_context(WORKSPACE)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::command))
            .relative()
            .size_full()
            .child(column)
            .children(settings)
    }
}

// ---------------------------------------------------------------- pieces

/// A form's row: what it asks, and the control that answers.
fn form_row(label: &'static str, control: impl IntoElement) -> gpui::AnyElement {
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
    let home = prefs::home().unwrap_or_else(|| PathBuf::from("."));
    let videos = if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    };
    home.join(videos).join("Kerf")
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
fn git_address(url: &str) -> bool {
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
