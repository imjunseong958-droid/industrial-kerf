//! Opening the editor's windows: a project in a window of its own, or the
//! launcher when there is none.

use crate::launcher::{LauncherEvent, LauncherView};
use crate::prefs::Prefs;
use crate::session::Session;
use crate::workspace::Workspace;
use gpui::{px, size, App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions};
use std::path::PathBuf;

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

/// Opens `project` in a window, or the launcher with none.
pub fn open(project: Option<PathBuf>, kerfd: Option<PathBuf>, cx: &mut App) {
    open_at(project, kerfd, None, cx);
}

/// Follows a kerf:// link: the recent project it names, at its moment. A
/// project not known here is said, and the launcher opens.
pub fn follow(text: &str, kerfd: Option<PathBuf>, cx: &mut App) {
    let recent = Prefs::load().recent;
    match crate::links::parse(text)
        .and_then(|link| crate::links::resolve(&link, &recent).map(|project| (project, link)))
    {
        Ok((project, link)) => open_at(Some(project), kerfd, Some(link), cx),
        Err(message) => {
            log::warn!("{text}: {message}");
            eprintln!("kerf-editor: {text}: {message}");
            open_at(None, kerfd, None, cx);
        }
    }
}

fn open_at(
    project: Option<PathBuf>,
    kerfd: Option<PathBuf>,
    link: Option<crate::links::Link>,
    cx: &mut App,
) {
    let mut prefs = Prefs::load();
    let Some(project) = project else {
        let options = options("Industrial Kerf", cx);
        let window = cx.open_window(options, |window, cx| {
            let launcher = cx.new(|cx| LauncherView::new(prefs, window, cx));
            let handle = window.window_handle();
            cx.subscribe(&launcher, move |_, event: &LauncherEvent, cx| match event {
                LauncherEvent::Open { project, .. } => {
                    open(Some(project.clone()), None, cx);
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            })
            .detach();
            launcher
        });
        if let Err(error) = window {
            log::error!("could not open the launcher: {error}");
        }
        return;
    };
    prefs.opened(&project);
    prefs.save();
    let title = project
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Industrial Kerf".into());
    let options = options(&title, cx);
    let window = cx.open_window(options, |window, cx| {
        let session = Session::open(Some(project), kerfd, prefs, window, cx);
        if let Some(link) = link {
            session.update(cx, |session, _| session.follow(link));
        }
        cx.new(|cx| Workspace::new(session, window, cx))
    });
    if let Err(error) = window {
        log::error!("could not open the editor: {error}");
    }
}
