//! Opening the editor's windows: a project in a window of its own, or the
//! launcher when there is none; following kerf:// links to them; and what
//! Kerf says on its own at launch (a crash since the last notice, a newer
//! release).
//!
//! One window per project: opening or following a link to a project that
//! is open raises its window. What answers questions over a window
//! (`Window::prompt`) is drawn in Kerf's own look off macOS ([`install_prompts`]).

use crate::launcher::{LauncherEvent, LauncherView};
use crate::motion;
use crate::prefs::Prefs;
use crate::session::{BridgeEvent, Session};
use crate::store::Change;
use crate::theme::{self, radius, text};
use crate::ui;
use crate::workspace::Workspace;
use gpui::{
    actions, div, prelude::*, px, size, AnimationExt, App, AppContext, Bounds, Context, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, Global, KeyBinding, PromptButton,
    PromptHandle, PromptLevel, PromptResponse, RenderablePromptHandle, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowHandle, WindowOptions,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The options every editor window opens with (a project's, the
/// launcher's, New Project's launcher): the screen edge to edge, titled
/// `title`; or, given `--window-size` ([`set_window_size`]), that size in
/// the middle of the screen.
pub fn options(title: &str, cx: &mut App) -> WindowOptions {
    let asked = cx.try_global::<WindowSize>().map(|asked| asked.0);
    let (width, height) = asked.unwrap_or((1600.0, 1000.0));
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    WindowOptions {
        // The editor fills the screen edge to edge, unless a size was asked for.
        window_bounds: Some(match asked {
            Some(_) => WindowBounds::Windowed(bounds),
            None => WindowBounds::Maximized(bounds),
        }),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            ..Default::default()
        }),
        app_id: Some("industrial-kerf".into()),
        window_min_size: Some(size(px(720.0), px(480.0))),
        ..Default::default()
    }
}

/// The size `--window-size` asked every window to open at.
struct WindowSize((f32, f32));

impl Global for WindowSize {}

/// Opens every window at `size` (points) rather than edge to edge, for a
/// capture. `main` calls it with `--window-size`.
pub fn set_window_size(size: Option<(f32, f32)>, cx: &mut App) {
    if let Some(size) = size {
        cx.set_global(WindowSize(size));
    }
}

/// `--window-size`'s WxH ("1600x1000"): both sides at least 1.
pub fn parse_window_size(text: &str) -> Option<(f32, f32)> {
    let (width, height) = text.trim().split_once(['x', 'X'])?;
    let side = |text: &str| {
        text.trim()
            .parse::<u32>()
            .ok()
            .filter(|side| (1..=16384).contains(side))
    };
    Some((side(width)? as f32, side(height)? as f32))
}

/// `path` (PATH, `:` between folders) with the folders Homebrew and
/// `~/.local` keep tools in after it, each once: an app opened from Finder
/// or the Dock is given no shell PATH, and so would not find git, gh or
/// git-lfs. `main` sets it on macOS before anything runs a tool.
pub fn path_with_tool_folders(path: &str, home: &Path) -> String {
    let mut folders: Vec<String> = path
        .split(':')
        .filter(|folder| !folder.is_empty())
        .map(str::to_owned)
        .collect();
    let local = home
        .join(".local")
        .join("bin")
        .to_string_lossy()
        .into_owned();
    for folder in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        local.as_str(),
    ] {
        if !folders.iter().any(|known| known == folder) {
            folders.push(folder.to_owned());
        }
    }
    folders.join(":")
}

// ---------------------------------------------------------------- windows

/// A project open in a window of its own.
#[derive(Clone)]
struct OpenProject {
    project: PathBuf,
    window: WindowHandle<Workspace>,
    session: WeakEntity<Session>,
}

/// The projects open, one window each.
#[derive(Default)]
struct Opened(Vec<OpenProject>);

impl Global for Opened {}

/// The window open on `project`, if one is (and still there).
fn find_open(project: &Path, cx: &mut App) -> Option<OpenProject> {
    let live: Vec<_> = cx
        .windows()
        .iter()
        .map(|window| window.window_id())
        .collect();
    let opened = cx.default_global::<Opened>();
    opened
        .0
        .retain(|open| live.contains(&open.window.window_id()) && open.session.upgrade().is_some());
    opened
        .0
        .iter()
        .find(|open| same_folder(&open.project, project))
        .cloned()
}

/// Whether `a` and `b` are one folder, however written.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Brings `open`'s window to the front, following `link` there.
fn raise(open: &OpenProject, link: Option<crate::links::Link>, cx: &mut App) {
    let _ = open
        .window
        .update(cx, |_, window, _| window.activate_window());
    if let Some(session) = open.session.upgrade() {
        session.update(cx, |session, cx| {
            if let Some(link) = link {
                session.follow(link);
            }
            cx.notify();
        });
    }
}

/// Opens `project` in a window, or the launcher with none. A project open
/// already has its window raised. With no `kerfd`, the one the editor was
/// started with ([`set_kerfd`]) is kept.
pub fn open(project: Option<PathBuf>, kerfd: Option<PathBuf>, cx: &mut App) {
    open_at(project, kerfd, None, cx);
}

/// The kerfd the editor was started with (its argument, or `KERFD_BIN`).
struct StartedKerfd(PathBuf);

impl Global for StartedKerfd {}

/// Keeps the kerfd the editor was started with, so every window opened
/// later (a project from the launcher, New and Open Project, a link) uses
/// it too. `main` calls it once, before the first window.
pub fn set_kerfd(kerfd: Option<PathBuf>, cx: &mut App) {
    if let Some(kerfd) = kerfd.filter(|kerfd| !kerfd.as_os_str().is_empty()) {
        cx.set_global(StartedKerfd(kerfd));
    }
}

/// The kerfd the editor was started with, if one was given.
pub fn started_kerfd(cx: &App) -> Option<PathBuf> {
    cx.try_global::<StartedKerfd>().map(|kerfd| kerfd.0.clone())
}

/// Follows a kerf:// link: the recent project it names, at its moment, in
/// its window if it is open. A link that cannot be followed is explained.
pub fn follow(text: &str, kerfd: Option<PathBuf>, cx: &mut App) {
    let recent = Prefs::load().recent;
    let found = crate::links::parse(text).and_then(|link| {
        let project = crate::links::resolve(&link, &recent)?;
        crate::links::openable(&project)?;
        Ok((project, link))
    });
    match found {
        Ok((project, link)) => open_at(Some(project), kerfd, Some(link), cx),
        Err(message) => {
            log::warn!("{text}: {message}");
            explain_link(text, &message, cx);
        }
    }
}

/// Tells the person why `link` was not followed: over a window open, else
/// in the launcher.
fn explain_link(link: &str, message: &str, cx: &mut App) {
    let window = cx.active_window().or_else(|| cx.windows().first().copied());
    let detail = format!("{message}\n\n{link}");
    let asked = window.and_then(|window| {
        window
            .update(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Warning,
                    "Review link",
                    Some(&detail),
                    &[PromptButton::ok("OK")],
                    cx,
                )
            })
            .ok()
    });
    if asked.is_none() {
        open_launcher(Some(format!("{link}: {message}")), started_kerfd(cx), cx);
    }
}

fn open_at(
    project: Option<PathBuf>,
    kerfd: Option<PathBuf>,
    link: Option<crate::links::Link>,
    cx: &mut App,
) {
    let kerfd = kerfd.or_else(|| started_kerfd(cx));
    let Some(project) = project else {
        open_launcher(None, kerfd, cx);
        return;
    };
    if let Some(open) = find_open(&project, cx) {
        raise(&open, link, cx);
        return;
    }
    let warnings = take_warnings(&project, cx);
    let mut prefs = Prefs::load();
    prefs.opened(&project);
    prefs.save();
    let title = project
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Industrial Kerf".into());
    let options = options(&title, cx);
    let mut made = None;
    let window = cx.open_window(options, |window, cx| {
        represent_project(window, &project);
        let session = Session::open(Some(project.clone()), kerfd, prefs, window, cx);
        // The account is the person's, whichever window heard of it.
        cx.subscribe(&session, |_, event: &BridgeEvent, cx| {
            if event.0["type"] == "cloud" {
                hear_cloud(&event.0, cx);
            }
        })
        .detach();
        if let Some(link) = link {
            session.update(cx, |session, _| session.follow(link));
        }
        say_when_up(&session, warnings, cx);
        made = Some(session.downgrade());
        cx.new(|cx| Workspace::new(session, window, cx))
    });
    match (window, made) {
        (Ok(window), Some(session)) => cx.default_global::<Opened>().0.push(OpenProject {
            project,
            window,
            session,
        }),
        (Err(error), _) => log::error!("could not open the editor: {error}"),
        (Ok(_), None) => {}
    }
}

/// The file a project's window represents: its project.kerf, as the Qt
/// editor's setWindowFilePath had it.
pub fn represented_file(project: &Path) -> PathBuf {
    project.join("project.kerf")
}

/// On macOS, has `window` represent `project`'s [`represented_file`], so
/// its title bar carries the file's proxy icon and the Window menu the
/// file, as the Qt editor's did; elsewhere nothing. A project's window
/// calls it as it opens: a window keeps its project.
pub fn represent_project(window: &Window, project: &Path) {
    #[cfg(target_os = "macos")]
    macos::represent(window, &represented_file(project));
    #[cfg(not(target_os = "macos"))]
    let _ = (window, project);
}

/// AppKit, through its safe bindings.
#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSView};
    use objc2_foundation::NSString;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    use std::path::Path;

    /// Has the NSWindow that holds `window`'s view (GPUI's, in its content
    /// view) represent `file`.
    pub(super) fn represent(window: &impl HasWindowHandle, file: &Path) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        let Some(main) = MainThreadMarker::new() else {
            return;
        };
        let view = appkit.ns_view.as_ptr().cast_const();
        let is_view =
            |candidate: &Retained<NSView>| Retained::as_ptr(candidate).cast::<c_void>() == view;
        let found = NSApplication::sharedApplication(main)
            .windows()
            .to_vec()
            .into_iter()
            .find(|ns_window| {
                ns_window.contentView().is_some_and(|content| {
                    is_view(&content) || content.subviews().to_vec().iter().any(&is_view)
                })
            });
        if let Some(ns_window) = found {
            ns_window.setRepresentedFilename(&NSString::from_str(&file.to_string_lossy()));
        }
    }
}

/// Opens the launcher, saying `problem` when there is one (a link that
/// could not be followed). `kerfd` is the one the editor was started with:
/// a project made from a preset imports its media with it, and the window
/// the launcher opens keeps it.
fn open_launcher(problem: Option<String>, kerfd: Option<PathBuf>, cx: &mut App) {
    let prefs = Prefs::load();
    let options = options("Industrial Kerf", cx);
    let window = cx.open_window(options, |window, cx| {
        let launcher = cx.new(|cx| LauncherView::new(prefs, window, cx).with_kerfd(kerfd.clone()));
        if let Some(problem) = problem {
            launcher.update(cx, |launcher, cx| launcher.say(problem, cx));
        }
        let handle = window.window_handle();
        cx.subscribe(&launcher, move |_, event: &LauncherEvent, cx| match event {
            LauncherEvent::Open { project, starts } => {
                // A folder that was no project yet brings its media in.
                crate::workspace::open_folder_with(project.clone(), *starts, kerfd.clone(), cx);
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        })
        .detach();
        launcher
    });
    if let Err(error) = window {
        log::error!("could not open the launcher: {error}");
    }
}

/// Renders the preview of the first project open once kerfd has it
/// (`--render-preview`): the registry's `sequence.renderPreview`, as View ▸
/// Render Preview runs it.
pub fn render_preview_when_up(cx: &mut App) {
    let Some(session) = cx
        .try_global::<Opened>()
        .and_then(|opened| opened.0.first())
        .and_then(|open| open.session.upgrade())
    else {
        return;
    };
    fn render(session: &Entity<Session>, cx: &mut App) {
        session.update(cx, |session, cx| {
            session.run_command("sequence.renderPreview", serde_json::json!({}));
            cx.notify();
        });
    }
    if session.read(cx).state.opened_or_failed() {
        render(&session, cx);
        return;
    }
    let mut waiting = true;
    cx.subscribe(&session, move |session, _: &Change, cx| {
        if waiting && session.read(cx).state.opened_or_failed() {
            waiting = false;
            render(&session, cx);
        }
    })
    .detach();
}

/// Opens the shot's graph `start` names (`--graph`, `--graph-node`) in the
/// first project's window, once its project has said that media
/// ([`crate::making::open_when_ready`]).
pub fn open_graph_when_up(start: crate::making::GraphStart, cx: &mut App) {
    let Some(open) = cx
        .try_global::<Opened>()
        .and_then(|opened| opened.0.first())
        .cloned()
    else {
        return;
    };
    let Some(session) = open.session.upgrade() else {
        return;
    };
    let window = open.window;
    crate::making::open_when_ready(
        start,
        &session,
        move |media, cx| {
            let _ = window.update(cx, |workspace, window, cx| {
                workspace.open_shot_graph(media, window, cx)
            });
        },
        cx,
    );
}

/// What making or cloning a project warned of, until its window is open.
#[derive(Default)]
struct Warnings(Vec<(PathBuf, Vec<String>)>);

impl Global for Warnings {}

/// Says `warnings` (from making or cloning `project`: Git LFS missing, a
/// push that failed) at the foot of its window once kerfd has it open, so
/// they are there to be read. The launcher calls it as it opens the
/// project; the window opened for it, now or next, takes them.
pub fn warn_when_open(project: &Path, warnings: Vec<String>, cx: &mut App) {
    if warnings.is_empty() {
        return;
    }
    match find_open(project, cx).and_then(|open| open.session.upgrade()) {
        Some(session) => say_when_up(&session, warnings, cx),
        None => cx
            .default_global::<Warnings>()
            .0
            .push((project.to_path_buf(), warnings)),
    }
}

/// The warnings kept for `project`, taken.
fn take_warnings(project: &Path, cx: &mut App) -> Vec<String> {
    let Some(kept) = cx.try_global::<Warnings>() else {
        return Vec::new();
    };
    if kept.0.is_empty() {
        return Vec::new();
    }
    let kept = &mut cx.default_global::<Warnings>().0;
    let mut taken = Vec::new();
    kept.retain(|(folder, warnings)| {
        if same_folder(folder, project) {
            taken.extend(warnings.iter().cloned());
            false
        } else {
            true
        }
    });
    taken
}

/// Says `warnings` in `session`'s window once its project is up (its
/// first snapshot), or kerfd could not be reached
/// ([`EditorState::opened_or_failed`](crate::store::EditorState::opened_or_failed)):
/// at once when it is already, else at the first change after which it is.
/// Not while it only connects: a notice goes after a while, and a large
/// project can take longer than that to open.
fn say_when_up(session: &Entity<Session>, warnings: Vec<String>, cx: &mut App) {
    if warnings.is_empty() {
        return;
    }
    fn say(session: &Entity<Session>, warnings: &[String], cx: &mut App) {
        session.update(cx, |session, cx| {
            for warning in warnings {
                session.state.notify(warning, true);
            }
            cx.emit(Change::Notice);
            cx.notify();
        });
    }
    if session.read(cx).state.opened_or_failed() {
        say(session, &warnings, cx);
        return;
    }
    let mut warnings = Some(warnings);
    cx.subscribe(session, move |session, _: &Change, cx| {
        if warnings.is_none() || !session.read(cx).state.opened_or_failed() {
            return;
        }
        if let Some(warnings) = warnings.take() {
            say(&session, &warnings, cx);
        }
    })
    .detach();
}

// ---------------------------------------------------------------- Kerf's account

/// The last `cloud` event any window heard: the person's Kerf account.
struct Cloud(Value);

impl Global for Cloud {}

/// What was last heard of the person's Kerf account (`cloud {access,
/// account?, message?}`); None before anything is.
pub fn cloud(cx: &App) -> Option<&Value> {
    cx.try_global::<Cloud>().map(|cloud| &cloud.0)
}

/// Keeps a `cloud` event for every window.
pub fn hear_cloud(event: &Value, cx: &mut App) {
    cx.set_global(Cloud(event.clone()));
}

// ---------------------------------------------------------------- at launch

/// Which part of Kerf a crash report is of, as a person says it.
pub fn component_name(component: &str) -> &'static str {
    match component {
        "kerfd" => "Kerf's project service (kerfd)",
        "kerf" => "the kerf command",
        _ => "the editor",
    }
}

/// What the crash notice says of `reports` (unseen, newest first).
pub fn crash_notice_text(reports: &[Value]) -> String {
    let Some(latest) = reports.first() else {
        return String::new();
    };
    let component = component_name(latest["component"].as_str().unwrap_or("editor"));
    let mut text = if reports.len() == 1 {
        format!("Kerf stopped unexpectedly: {component} hit an error it could not recover from.")
    } else {
        format!(
            "Kerf stopped unexpectedly {} times since it last said so. The latest was in {component}.",
            reports.len()
        )
    };
    if let Some(message) = latest["message"]
        .as_str()
        .and_then(|message| message.lines().next())
        .filter(|message| !message.trim().is_empty())
    {
        text.push_str(&format!("\n\n“{}”", message.trim()));
    }
    text.push_str(
        "\n\nA report of each is in the crash folder, on this computer only. Attaching one to a bug report shows where it happened.",
    );
    text
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// The window a notice goes over: the one in front, else the first.
fn front_window(cx: &App) -> Option<gpui::AnyWindowHandle> {
    cx.active_window().or_else(|| cx.windows().first().copied())
}

/// What Kerf says on its own at launch, once a window is open: a crash
/// since the last notice, then, with update notices on and the last
/// question to GitHub 20 h ago or more, a newer release not skipped.
pub fn launch_notices(cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Ok(Some((answer, folder))) = cx.update(crash_notice) {
            if matches!(answer.await, Ok(1)) {
                let _ = cx.update(|cx| {
                    let _ = std::fs::create_dir_all(&folder);
                    cx.reveal_path(&folder);
                });
            }
        }
        let due = cx
            .update(|_| {
                let mut prefs = Prefs::load();
                let now = now_ms();
                if !prefs.update_due(now) {
                    return false;
                }
                // Counted when asked, so a failing network is not asked again all day.
                prefs.update_checked_ms = now;
                prefs.save();
                true
            })
            .unwrap_or(false);
        if !due {
            return;
        }
        let answer = cx
            .background_executor()
            .spawn(async { kerf_bridge::update_check() })
            .await;
        let Ok(Some((answer, version, url))) = cx.update(|cx| offer_update(&answer, cx)) else {
            return;
        };
        match answer.await {
            Ok(0) => {
                let _ = cx.update(|cx| cx.open_url(&url));
            }
            Ok(1) => {
                let _ = cx.update(|_| {
                    let mut prefs = Prefs::load();
                    prefs.update_skipped = version;
                    prefs.save();
                });
            }
            _ => {}
        }
    })
    .detach();
}

/// Says so once when Kerf crashed since the last notice: Close (0) or Show
/// Crash Folder (1), and the folder.
fn crash_notice(
    cx: &mut App,
) -> Option<(
    impl std::future::Future<Output = Result<usize, impl Sized>>,
    PathBuf,
)> {
    let listed = kerf_bridge::crash_reports();
    let mut prefs = Prefs::load();
    let unseen = prefs.unseen_crashes(&listed);
    if unseen.is_empty() {
        return None;
    }
    let folder = PathBuf::from(listed["folder"].as_str()?);
    let window = front_window(cx)?;
    prefs.saw_crashes(&unseen);
    prefs.save();
    let text = crash_notice_text(&unseen);
    let answer = window
        .update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Warning,
                "Kerf stopped unexpectedly",
                Some(&text),
                // Esc answers Close: the folder opens only when asked for.
                &[
                    PromptButton::cancel("Close"),
                    PromptButton::new("Show Crash Folder"),
                ],
                cx,
            )
        })
        .ok()?;
    Some((answer, folder))
}

/// Offers the release `answer` names when it is newer and not skipped:
/// Download (0), Skip This Version (1) or Not Now (2).
fn offer_update(
    answer: &Value,
    cx: &mut App,
) -> Option<(
    impl std::future::Future<Output = Result<usize, impl Sized>>,
    String,
    String,
)> {
    let (version, url) = Prefs::load().update_offer(answer)?;
    let window = front_window(cx)?;
    let detail = format!(
        "Industrial Kerf {version} is available. Download it from its release page, and unpack it over this one or beside it."
    );
    let answer = window
        .update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Info,
                "A newer Kerf is out",
                Some(&detail),
                &[
                    PromptButton::ok("Download"),
                    PromptButton::new("Skip This Version"),
                    PromptButton::cancel("Not Now"),
                ],
                cx,
            )
        })
        .ok()?;
    Some((answer, version, url))
}

// ---------------------------------------------------------------- prompts

actions!(prompt, [PromptDefault, PromptCancel]);

/// The key context a question over the window answers in.
const PROMPT: &str = "KerfPrompt";
/// A question's width.
const PROMPT_WIDTH: f32 = 440.0;

/// Questions over a window (`Window::prompt`) in Kerf's look: off macOS,
/// where the system draws none of its own.
pub fn install_prompts(cx: &mut App) {
    if cfg!(target_os = "macos") {
        return;
    }
    cx.set_prompt_builder(kerf_prompt);
}

fn kerf_prompt(
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    // Binding a keymap clears these: bound again whenever they are missing.
    if !crate::settings::escape_bound(cx, &PromptCancel) {
        cx.bind_keys([
            KeyBinding::new("escape", PromptCancel, Some(PROMPT)),
            KeyBinding::new("enter", PromptDefault, Some(PROMPT)),
        ]);
    }
    static ASKED: AtomicU64 = AtomicU64::new(0);
    let view = cx.new(|cx| KerfPrompt {
        level,
        message: message.to_owned(),
        detail: detail.map(str::to_owned),
        actions: actions.to_vec(),
        focus: cx.focus_handle(),
        asked: ASKED.fetch_add(1, Ordering::Relaxed),
    });
    handle.with_view(view, window, cx)
}

/// A question over the dimmed window, its answers in a row at its foot:
/// the first, the default, at the end and filled.
struct KerfPrompt {
    level: PromptLevel,
    message: String,
    detail: Option<String>,
    actions: Vec<PromptButton>,
    focus: FocusHandle,
    /// Which question this is, so each rises once.
    asked: u64,
}

impl EventEmitter<PromptResponse> for KerfPrompt {}

impl Focusable for KerfPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KerfPrompt {
    /// Esc: the Cancel answer, else the last.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        let index = self
            .actions
            .iter()
            .position(|action| matches!(action, PromptButton::Cancel(_)))
            .unwrap_or(self.actions.len().saturating_sub(1));
        cx.emit(PromptResponse(index));
    }
}

impl Render for KerfPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mark = match self.level {
            PromptLevel::Critical => theme::neg(),
            PromptLevel::Warning => theme::warn(),
            PromptLevel::Info => theme::accent(),
        };
        let answers = self
            .actions
            .iter()
            .enumerate()
            .rev()
            .map(|(index, action)| {
                let button = ui::button(("prompt-answer", index), action.label().clone())
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(index))));
                if index == 0 {
                    button.primary()
                } else {
                    button
                }
            });
        let body = div()
            .w(px(PROMPT_WIDTH))
            .flex()
            .flex_col()
            .gap(px(theme::space::S12))
            .p(px(20.0))
            .rounded(px(radius::PANEL))
            .bg(theme::chrome())
            .border_1()
            .border_color(theme::line2())
            .shadow_2xl()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(ui::dot(mark, 8.0))
                    .child(ui::label(self.message.clone(), text::TITLE, theme::ink())),
            )
            .children(self.detail.clone().map(|detail| {
                div().flex().flex_col().gap(px(theme::space::S4)).children(
                    detail
                        .split("\n\n")
                        .map(|paragraph| ui::label(paragraph.to_owned(), text::BODY, theme::ink2()))
                        .collect::<Vec<_>>(),
                )
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .pt(px(theme::space::S4))
                    .children(answers),
            )
            // It comes up into place, as what one goes into does.
            .with_animation(
                ("prompt-rise", self.asked),
                motion::animation(theme::motion::PAGE),
                |body, t| {
                    body.relative()
                        .top(px(theme::motion::PAGE_RISE * (1.0 - t)))
                        .opacity(0.2 + 0.8 * t)
                },
            );
        div()
            .key_context(PROMPT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &PromptDefault, _, cx| cx.emit(PromptResponse(0))))
            .on_action(cx.listener(|this, _: &PromptCancel, _, cx| this.cancel(cx)))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .font(theme::ui_font(FontWeight::NORMAL))
            // The window dims at once, and nothing under the dim answers.
            .bg(theme::with_alpha(gpui::rgb(0x000000), theme::motion::SCRIM))
            .occlude()
            .child(body)
    }
}
