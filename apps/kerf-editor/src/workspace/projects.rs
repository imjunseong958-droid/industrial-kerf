//! Another project in place of this window: opened from a folder (asked
//! about first when it is no project yet), New Project's launcher, File ▸
//! Open Recent, another folder's conversation in the sidebar; and media
//! brought in from files chosen or from a folder that becomes a project.

use super::asked::Confirm;
use super::Workspace;
use crate::session::Session;
use gpui::{prelude::*, App, Context, PathPromptOptions, Window, WindowBounds};
use std::path::{Path, PathBuf};

impl Workspace {
    /// File ▸ Import Media…: the files chosen come into the project's media.
    /// No chooser to ask (no portal), or a name kerfd cannot be sent, says so.
    pub(super) fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("Import".into()),
            },
        );
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let answer = chosen.await;
            let _ = session.update(cx, |session, cx| {
                if let Some(paths) = chosen_paths(answer, session) {
                    for path in &paths {
                        session.import(path, false);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a folder in place of this project. One that is not a project
    /// yet is asked about first; this project is open already.
    pub(super) fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = crate::presets::choose_paths(
            cx,
            PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Open".into()),
            },
        );
        let current = self.session.read(cx).project.clone();
        let session = self.session.downgrade();
        cx.spawn_in(window, async move |this, cx| {
            let answer = chosen.await;
            let Ok(Some(paths)) = session.update(cx, |session, cx| {
                cx.notify();
                chosen_paths(answer, session)
            }) else {
                return;
            };
            let Some(project) = paths.into_iter().next() else {
                return;
            };
            if current
                .as_deref()
                .is_some_and(|current| crate::folders::same_folder(current, &project))
            {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| this.open_chosen(project, window, cx));
        })
        .detach();
    }

    /// Opens `project` in place of this window: one that is not a project
    /// yet is asked about first; this window's own project stays as it is.
    pub(super) fn open_chosen(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if crate::folders::needs_start(&project) {
            self.ask_to_start(project, window, cx);
        } else {
            self.replace_with(project, false, window, cx);
        }
    }

    /// Opens `project` in a window of its own with this window's kerfd,
    /// and closes this one: another project takes its place.
    /// This window's own project stays open.
    fn replace_with(
        &mut self,
        project: PathBuf,
        starts: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
        if session
            .project
            .as_deref()
            .is_some_and(|current| crate::folders::same_folder(current, &project))
        {
            window.activate_window();
            return;
        }
        let kerfd = session.kerfd().map(Path::to_path_buf);
        open_folder_in(project, starts, kerfd, Some(window.window_bounds()), cx);
        window.remove_window();
    }

    /// New Project: the launcher, in a window of its own. What it opens
    /// takes this window's place, with this window's kerfd; closed with
    /// nothing chosen, this window stays.
    pub(super) fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let kerfd = session.kerfd().map(Path::to_path_buf);
        let current = session.project.clone();
        let replaced = window.window_handle();
        let prefs = crate::prefs::Prefs::load();
        // Over this window, where it is; what it opens takes this window's place.
        let options = crate::app::launcher_options(Some(window.window_bounds()), cx);
        let opened = cx.open_window(options, move |launcher_window, cx| {
            // What it makes from a preset brings its media in with this kerfd too.
            let launcher = cx.new(|cx| {
                crate::launcher::LauncherView::new(prefs, launcher_window, cx)
                    .with_kerfd(kerfd.clone())
                    .over_editor()
            });
            let handle = launcher_window.window_handle();
            cx.subscribe(
                &launcher,
                move |_, event: &crate::launcher::LauncherEvent, cx| match event {
                    crate::launcher::LauncherEvent::Open {
                        project,
                        starts,
                        warnings,
                    } => {
                        crate::app::warn_when_open(project, warnings.clone(), cx);
                        let here = current
                            .as_deref()
                            .is_some_and(|current| crate::folders::same_folder(current, project));
                        if !here {
                            let place = replaced
                                .update(cx, |_, window, _| window.window_bounds())
                                .ok();
                            open_folder_in(project.clone(), *starts, kerfd.clone(), place, cx);
                            let _ = replaced.update(cx, |_, window, _| window.remove_window());
                        } else {
                            let _ = replaced.update(cx, |_, window, _| window.activate_window());
                        }
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    }
                    // Given up: this window comes back and the launcher's
                    // goes, leaving the person where they were. With this
                    // window closed meanwhile, the launcher stays, the only
                    // window left.
                    crate::launcher::LauncherEvent::Cancel => {
                        if replaced
                            .update(cx, |_, window, _| window.activate_window())
                            .is_ok()
                        {
                            let _ = handle.update(cx, |_, window, _| window.remove_window());
                        }
                    }
                },
            )
            .detach();
            launcher
        });
        if let Err(error) = opened {
            log::error!("could not open the launcher: {error}");
        }
    }

    /// "“X” is not a project yet. Start one here, with its media?"
    fn ask_to_start(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() {
            return;
        }
        let name = project
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| project.display().to_string());
        let kerfd = self.session.read(cx).kerfd().map(Path::to_path_buf);
        let confirm = cx.new(|cx| {
            Confirm::new(
                cx,
                format!("“{name}” is not a project yet."),
                Some("Start one here, with its media?"),
                "Start",
                false,
                true,
                move |window, cx| {
                    open_folder_in(project, true, kerfd, Some(window.window_bounds()), cx);
                    window.remove_window();
                },
            )
        });
        self.open_modal(confirm, window, cx);
    }
}

/// The paths a file chooser gave; None, with a notice in `session`'s window
/// saying why where there is something to say (no chooser could open, a
/// name kerfd cannot be sent), when there are none to use.
fn chosen_paths(answer: crate::presets::Picked, session: &mut Session) -> Option<Vec<PathBuf>> {
    use crate::presets::Picked;
    match answer {
        Picked::Paths(paths) => Some(paths),
        Picked::Failed(error) => {
            session
                .state
                .notify(&format!("No file chooser could open: {error}"), true);
            None
        }
        Picked::Unusable(words) => {
            session.state.notify(&words, true);
            None
        }
        Picked::Nothing => None,
    }
}

/// The media files a folder that becomes a project brings in.
pub const FOLDER_MEDIA: [&str; 7] = ["mov", "mp4", "m4v", "mkv", "webm", "wav", "mp3"];

/// The media files directly in `folder`, by name.
pub fn folder_media(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    FOLDER_MEDIA.contains(&extension.to_ascii_lowercase().as_str())
                })
        })
        .collect();
    files.sort();
    files
}

/// The window's title: the project, and its sequence when that is named
/// otherwise ("Atlas · 30 s spot v7").
pub fn window_title(project: &str, timeline: &crate::model::Timeline) -> String {
    let sequence = timeline
        .sequence
        .as_deref()
        .and_then(|id| timeline.sequences.iter().find(|sequence| sequence.id == id))
        .map(|sequence| sequence.name.trim())
        .unwrap_or_default();
    if sequence.is_empty() || sequence == project {
        project.to_owned()
    } else if project.is_empty() {
        sequence.to_owned()
    } else {
        format!("{project} · {sequence}")
    }
}

/// Opens `project` in a window of its own. `starts` when it was no project
/// yet: once kerfd has it, its folder's media comes in (the launcher's
/// `LauncherEvent::Open { starts }` means the same).
pub fn open_folder(project: PathBuf, starts: bool, cx: &mut App) {
    open_folder_with(project, starts, None, cx);
}

/// [`open_folder`] with the kerfd the project is opened with (None finds
/// one as at launch): a window opening another project keeps its own.
pub fn open_folder_with(project: PathBuf, starts: bool, kerfd: Option<PathBuf>, cx: &mut App) {
    open_folder_in(project, starts, kerfd, None, cx);
}

/// [`open_folder_with`], in `place`: the bounds of the window it takes
/// the place of (`window.window_bounds()`), so it opens where that one
/// was, at its size, maximized if it was, and the window the person
/// placed does not move.
pub fn open_folder_in(
    project: PathBuf,
    starts: bool,
    kerfd: Option<PathBuf>,
    place: Option<WindowBounds>,
    cx: &mut App,
) {
    crate::app::open_in_place(project.clone(), kerfd, place, cx);
    if !starts {
        return;
    }
    for handle in cx.windows() {
        let Some(workspace) = handle.downcast::<Workspace>() else {
            continue;
        };
        // The window being updated (this one) answers Err, and is left be.
        let _ = workspace.update(cx, |workspace, _, cx| {
            let session = workspace.session.read(cx);
            let fresh = !session.state.has_snapshot;
            let here = session
                .project
                .as_deref()
                .is_some_and(|open| crate::folders::same_folder(open, &project));
            if fresh && here && workspace.start_media.is_none() {
                workspace.start_media = Some(project.clone());
            }
        });
    }
}

/// Opens conversation `id` (None: a new one) in the window whose project
/// is `folder`, once what opens it there has run (`OpenRecent`, which
/// opens the folder's project in place of this window, or brings its own
/// window to the front).
pub fn open_conversation_in(folder: PathBuf, id: Option<String>, cx: &mut App) {
    cx.defer(move |cx| {
        for handle in cx.windows() {
            let Some(workspace) = handle.downcast::<Workspace>() else {
                continue;
            };
            let _ = workspace.update(cx, |workspace, _, cx| {
                let session = workspace.session().clone();
                session.update(cx, |session, cx| {
                    let here = session
                        .project
                        .as_deref()
                        .is_some_and(|open| crate::folders::same_folder(open, &folder));
                    if here {
                        session.open_conversation(id.clone());
                        cx.notify();
                    }
                });
            });
        }
    });
}
