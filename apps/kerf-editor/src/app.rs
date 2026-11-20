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
use crate::theme::{self, radius, text};
use crate::ui;
use crate::workspace::Workspace;
use gpui::{
    actions, div, prelude::*, px, size, AnimationExt, App, AppContext, Bounds, Context,
    EventEmitter, FocusHandle, Focusable, FontWeight, Global, KeyBinding, PromptButton,
    PromptHandle, PromptLevel, PromptResponse, RenderablePromptHandle, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowHandle, WindowOptions,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn options(title: &str, cx: &mut App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1600.0), px(1000.0)), cx);
    WindowOptions {
        // The editor fills the screen edge to edge.
        window_bounds: Some(WindowBounds::Maximized(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_owned().into()),
            ..Default::default()
        }),
        app_id: Some("industrial-kerf".into()),
        window_min_size: Some(size(px(720.0), px(480.0))),
        ..Default::default()
    }
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
/// already has its window raised.
pub fn open(project: Option<PathBuf>, kerfd: Option<PathBuf>, cx: &mut App) {
    open_at(project, kerfd, None, cx);
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
        open_launcher(Some(format!("{link}: {message}")), cx);
    }
}

fn open_at(
    project: Option<PathBuf>,
    kerfd: Option<PathBuf>,
    link: Option<crate::links::Link>,
    cx: &mut App,
) {
    let Some(project) = project else {
        open_launcher(None, cx);
        return;
    };
    if let Some(open) = find_open(&project, cx) {
        raise(&open, link, cx);
        return;
    }
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

/// Opens the launcher, saying `problem` when there is one (a link that
/// could not be followed).
fn open_launcher(problem: Option<String>, cx: &mut App) {
    let prefs = Prefs::load();
    let options = options("Industrial Kerf", cx);
    let window = cx.open_window(options, |window, cx| {
        let launcher = cx.new(|cx| LauncherView::new(prefs, window, cx));
        if let Some(problem) = problem {
            launcher.update(cx, |launcher, cx| launcher.say(problem, cx));
        }
        let handle = window.window_handle();
        cx.subscribe(&launcher, move |_, event: &LauncherEvent, cx| match event {
            LauncherEvent::Open { project, starts } => {
                // A folder that was no project yet brings its media in.
                crate::workspace::open_folder(project.clone(), *starts, cx);
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
