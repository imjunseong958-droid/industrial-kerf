//! `kerf-editor [project] [kerfd]`: Industrial Kerf's editor.
//!
//! The project may also come as `KERF_PROJECT_DIR` and kerfd as `KERFD_BIN`.
//! Without a project the editor opens on the one last worked on, else the
//! launcher. `--serve-mcp --dir <project>` serves the project's MCP tools on
//! stdio instead of opening a window (Claude Code is given the editor
//! itself as its MCP server).
//!
//! For captures and test runs: `--quit-after-ms <ms>` quits after that
//! long; `--settings-dir <dir>` keeps the settings in a folder of their
//! own, and keys out of the system's keychain; `--window-size WxH` opens
//! the first window at that size rather than edge to edge (the windows
//! after it open as they would); `--render-preview` renders the project's
//! preview once it is open; `--graph <media>` opens that shot's graph once
//! the project has it, on `--graph-node <id>` if given. A smoke run (one
//! that quits on its own, or asks for `--screenshot`) opens no project it
//! was not given; neither it nor a preview says anything on its own at
//! launch. `--screenshot <file>` writes a PNG of the first window as the
//! run quits (after `--quit-after-ms`, else half a second, once the window
//! has drawn): read back from the X server on X11, by `screencapture` on
//! macOS; on Wayland and Windows it says on stderr that it cannot.
//!
//! A kerf:// link may come anywhere among the arguments. Off macOS, an
//! editor started for links alone hands them to the editor already open
//! (its link inbox) and ends.

use kerf_editor::{app, icons, keymap, links, motion, prefs, prefs::Prefs, session, theme};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

struct Arguments {
    /// kerf:// links to follow, the first instead of a project.
    links: Vec<String>,
    project: Option<PathBuf>,
    /// The project was named among the arguments (not only in the
    /// environment): links are then followed beside it, never handed over.
    named_project: bool,
    kerfd: Option<PathBuf>,
    quit_after: Option<Duration>,
    /// `--settings-dir`'s folder.
    settings_dir: Option<PathBuf>,
    window_size: Option<(f32, f32)>,
    screenshot: Option<PathBuf>,
    render_preview: bool,
    /// `--graph` and `--graph-node`: the shot's graph to open.
    graph: Option<kerf_editor::making::GraphStart>,
}

impl Arguments {
    /// A run that quits on its own or captures: nobody is there to answer
    /// it, and nothing of the person's is opened.
    fn smoke(&self) -> bool {
        self.quit_after.is_some() || self.screenshot.is_some()
    }

    /// A run that says nothing on its own at launch (no crash notice, no
    /// question to GitHub) and neither hands links over nor takes them: a
    /// smoke run, or one that renders a preview.
    fn quiet(&self) -> bool {
        self.smoke() || self.render_preview
    }
}

const USAGE: &str = "usage: kerf-editor [project | kerf://link…] [kerfd] [--quit-after-ms MS] [--settings-dir DIR] [--window-size WxH] [--render-preview] [--screenshot FILE] [--graph MEDIA [--graph-node ID]]\n       kerf-editor --serve-mcp --dir PROJECT";

fn arguments(raw: &[String]) -> Result<Arguments, String> {
    let mut raw = raw.to_vec();
    let graph = kerf_editor::making::graph_start(&mut raw)?;
    let mut positional = Vec::new();
    let mut links = Vec::new();
    let mut quit_after = None;
    let mut settings_dir = None;
    let mut window_size = None;
    let mut screenshot = None;
    let mut render_preview = false;
    let mut iter = raw.iter();
    while let Some(argument) = iter.next() {
        // `--option=value` as well as `--option value`.
        let (option, inline) = match argument.split_once('=') {
            Some((option, value)) if option.starts_with("--") => (option, Some(value.to_owned())),
            _ => (argument.as_str(), None),
        };
        let mut value = |what: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| iter.next().cloned())
                .ok_or_else(|| format!("{option} needs {what}"))
        };
        match option {
            "--settings-dir" => {
                let folder = value("a folder")?;
                let folder = std::path::absolute(&folder).unwrap_or_else(|_| folder.into());
                // Before anything reads them: the settings there, and no keychain.
                std::env::set_var("KERF_EDITOR_PREFS", folder.join("editor.json"));
                std::env::set_var("KERF_SECRETS", "off");
                settings_dir = Some(folder);
            }
            "--quit-after-ms" => {
                let ms = value("milliseconds")?;
                let ms: u64 = ms.parse().map_err(|_| format!("not milliseconds: {ms}"))?;
                quit_after = Some(Duration::from_millis(ms));
            }
            "--window-size" => {
                let size = value("a size, WxH")?;
                window_size = Some(
                    app::parse_window_size(&size)
                        .ok_or_else(|| format!("not a size (WxH, as 1600x1000): {size}"))?,
                );
            }
            "--screenshot" => screenshot = Some(PathBuf::from(value("a file")?)),
            "--render-preview" => render_preview = true,
            "--help" | "-h" => return Err(USAGE.into()),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other if links::is_link(other) => links.push(other.to_owned()),
            other => positional.push(PathBuf::from(other)),
        }
    }
    let named_project = !positional.is_empty();
    let mut positional = positional.into_iter();
    let project = positional.next().or_else(|| {
        std::env::var_os("KERF_PROJECT_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    });
    let kerfd = positional.next().or_else(|| {
        std::env::var_os("KERFD_BIN")
            .filter(|bin| !bin.is_empty())
            .map(PathBuf::from)
    });
    Ok(Arguments {
        links,
        project: project.map(|project| std::path::absolute(&project).unwrap_or(project)),
        named_project,
        kerfd,
        quit_after,
        settings_dir,
        window_size,
        screenshot,
        render_preview,
        graph,
    })
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = kerf_bridge::serve_mcp(&raw) {
        std::process::exit(code);
    }
    // Before any thread starts, and for every tool the editor and kerfd
    // run: Finder and the Dock give an app no shell PATH.
    if cfg!(target_os = "macos") {
        if let Some(home) = prefs::home() {
            let path = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", app::path_with_tool_folders(&path, &home));
        }
    }
    // Settings someone else put in place (a test's) are theirs alone.
    let settings_given = std::env::var_os("KERF_EDITOR_PREFS").is_some_and(|path| !path.is_empty());
    let arguments = match arguments(&raw) {
        Ok(arguments) => arguments,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    kerf_bridge::crash_install();
    motion::init_from_env();
    // Before anything reads them, the first time: the settings the editor
    // before 0.1.5 (Qt) kept, so an update loses none of them.
    if !settings_given || arguments.settings_dir.is_some() {
        let taken = prefs::import_qt_once(arguments.settings_dir.as_deref());
        if !taken.is_empty() {
            log::info!("took up the Qt editor's settings: {}", taken.join(", "));
        }
    }
    let mut prefs = Prefs::load();
    if prefs.reduce_motion {
        motion::set_reduced(true);
    }
    theme::set_faces(prefs.face, prefs.reading_face, prefs.prose_step);
    let smoke = arguments.smoke();

    // Off macOS each link followed starts an editor: one already open takes
    // the links, and this one, started for them alone, ends before it shows
    // anything. A smoke run or a preview neither hands links over nor takes
    // them.
    let quiet = arguments.quiet();
    let takes_links = !quiet;
    let inbox = if takes_links && !cfg!(target_os = "macos") {
        links::inbox_here(&prefs.inbox_id())
    } else {
        None
    };
    if let Some(address) = &inbox {
        if !arguments.links.is_empty()
            && !arguments.named_project
            && links::hand_off(address, &arguments.links, Duration::from_secs(2))
        {
            return;
        }
    }

    // The first link opens its project, unless a project is named: then
    // every link is followed once that project's window is open.
    let mut later = arguments.links;
    let link = (arguments.project.is_none() && !later.is_empty()).then(|| later.remove(0));
    // Without a project, the one last worked on; not in a smoke run, whose
    // picture is the same on every machine and opens nothing of the person's.
    let project = arguments.project.or_else(|| {
        (link.is_none() && !smoke)
            .then(|| {
                prefs
                    .recent
                    .iter()
                    .find(|project| project.join("project.kerf").is_file())
                    .cloned()
            })
            .flatten()
    });
    let render_preview = arguments.render_preview && project.is_some();
    let graph = arguments
        .graph
        .filter(|_| project.is_some() || link.is_some());

    // Links the system hands the running editor (macOS), and those handed
    // to the inbox (elsewhere), taken up below.
    static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let take = |links: Vec<String>| {
        if let Ok(mut opened) = OPENED.lock() {
            opened.extend(links.into_iter().filter(|link| links::is_link(link)));
        }
    };
    let application = gpui::Application::new().with_assets(icons::Assets);
    application.on_open_urls(take);
    // Kept while the editor runs; dropped, the next editor listens there.
    let _inbox = inbox
        .as_ref()
        .and_then(|address| links::listen(address, take));
    // Links given beside a project are followed once its window is open.
    take(later);
    let kerfd = arguments.kerfd;
    let window_size = arguments.window_size;
    let quit_after = arguments.quit_after;
    let screenshot = arguments.screenshot;
    application.run(move |cx| {
        if let Err(error) = cx.text_system().add_fonts(theme::bundled_fonts()) {
            log::warn!("bundled fonts did not load: {error}");
        }
        app::install_prompts(cx);
        keymap::bind(prefs.keymap.wire(), cx);
        // The editor is its windows: the last one closed, it quits.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        // Every window opened later keeps the kerfd given here; the first
        // alone takes the size.
        session::set_kerfd(kerfd.clone(), cx);
        app::set_window_size(window_size, cx);
        match link {
            Some(link) => app::follow(&link, kerfd.clone(), cx),
            None => app::open(project, kerfd.clone(), cx),
        }
        if render_preview {
            app::render_preview_when_up(cx);
        }
        if let Some(start) = graph {
            app::open_graph_when_up(start, cx);
        }
        // A smoke run or a preview says nothing on its own: a notice
        // would cover what it renders, and nobody is there to answer it.
        if !quiet {
            app::launch_notices(cx);
        }
        cx.spawn(async move |cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let opened = OPENED
                .lock()
                .map(|mut opened| std::mem::take(&mut *opened))
                .unwrap_or_default();
            for link in opened {
                if cx
                    .update(|cx| app::follow(&link, kerfd.clone(), cx))
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
        cx.activate(true);
        if smoke {
            let after = quit_after.unwrap_or(Duration::from_millis(500));
            match screenshot {
                Some(file) => app::screenshot_then_quit(file, after, cx),
                None => cx
                    .spawn(async move |cx| {
                        cx.background_executor().timer(after).await;
                        // Refused only once the editor has ended already.
                        let _ = cx.update(|cx| cx.quit());
                    })
                    .detach(),
            }
        }
    });
}
