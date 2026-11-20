//! `kerf-editor [project] [kerfd]`: Industrial Kerf's editor.
//!
//! The project may also come as `KERF_PROJECT_DIR` and kerfd as `KERFD_BIN`.
//! Without a project the launcher opens. `--serve-mcp --dir <project>`
//! serves the project's MCP tools on stdio instead of opening a window
//! (Claude Code is given the editor itself as its MCP server).
//! `--quit-after-ms <ms>` quits after that long, for a smoke run (which
//! says nothing on its own at launch). `--settings-dir <dir>` keeps the
//! settings in a folder of their own, and keys out of the system's keychain.
//! A kerf:// link may come anywhere among the arguments.

use kerf_editor::{app, commands, icons, links, motion, prefs::Prefs, theme};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

struct Arguments {
    /// kerf:// links to follow, the first instead of a project.
    links: Vec<String>,
    project: Option<PathBuf>,
    kerfd: Option<PathBuf>,
    quit_after: Option<Duration>,
}

fn arguments(raw: &[String]) -> Result<Arguments, String> {
    let mut positional = Vec::new();
    let mut links = Vec::new();
    let mut quit_after = None;
    let mut iter = raw.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--settings-dir" => {
                let folder = iter.next().ok_or("--settings-dir needs a folder")?;
                let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.into());
                // Before anything reads them: the settings there, and no keychain.
                std::env::set_var("KERF_EDITOR_PREFS", folder.join("editor.json"));
                std::env::set_var("KERF_SECRETS", "off");
            }
            "--quit-after-ms" => {
                let value = iter.next().ok_or("--quit-after-ms needs milliseconds")?;
                let ms: u64 = value.parse().map_err(|_| format!("not milliseconds: {value}"))?;
                quit_after = Some(Duration::from_millis(ms));
            }
            "--help" | "-h" => {
                return Err("usage: kerf-editor [project | kerf://link…] [kerfd] [--quit-after-ms MS] [--settings-dir DIR]\n       kerf-editor --serve-mcp --dir PROJECT".into())
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other if links::is_link(other) => links.push(other.to_owned()),
            other => positional.push(PathBuf::from(other)),
        }
    }
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
        kerfd,
        quit_after,
    })
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = kerf_bridge::serve_mcp(&raw) {
        std::process::exit(code);
    }
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
    let prefs = Prefs::load();
    if prefs.reduce_motion {
        motion::set_reduced(true);
    }
    theme::set_face(prefs.face, prefs.prose_step);
    // Without a project, the one last worked on.
    // The first link opens its project, unless a project is named: then
    // every link is followed once that project's window is open.
    let mut later = arguments.links;
    let link = (arguments.project.is_none() && !later.is_empty()).then(|| later.remove(0));
    let project = arguments.project.or_else(|| {
        link.is_none()
            .then(|| {
                prefs
                    .recent
                    .iter()
                    .find(|project| project.is_dir())
                    .cloned()
            })
            .flatten()
    });

    // Links the system hands the running editor (macOS), taken up below.
    static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let application = gpui::Application::new().with_assets(icons::Assets);
    application.on_open_urls(|urls| {
        if let Ok(mut opened) = OPENED.lock() {
            opened.extend(urls.into_iter().filter(|url| links::is_link(url)));
        }
    });
    // Links given beside a project are followed once its window is open.
    if let Ok(mut opened) = OPENED.lock() {
        opened.extend(later);
    }
    application.run(move |cx| {
        if let Err(error) = cx.text_system().add_fonts(theme::bundled_fonts()) {
            log::warn!("bundled fonts did not load: {error}");
        }
        app::install_prompts(cx);
        commands::bind(prefs.keymap.wire(), cx);
        // The editor is its windows: the last one closed, it quits.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let kerfd = arguments.kerfd;
        // Every window opened later keeps the kerfd given here.
        app::set_kerfd(kerfd.clone(), cx);
        match link {
            Some(link) => app::follow(&link, kerfd.clone(), cx),
            None => app::open(project, kerfd.clone(), cx),
        }
        // A smoke run says nothing on its own.
        if arguments.quit_after.is_none() {
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
        if let Some(after) = arguments.quit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(after).await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}
