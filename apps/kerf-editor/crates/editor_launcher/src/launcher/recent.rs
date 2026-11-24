//! The projects opened before: each shows at once by its folder, its
//! name, branch and presets following as they are read; their rows fade up
//! one after another; Up and Down choose among them, Return or a press
//! opens one, and one no longer there can be forgotten.

use super::keys::{Stop, CONTROL};
use super::plan::{folder_of, home_relative};
use super::{staggered, LauncherView};
use editor_ui::icons::Icon;
use editor_ui::theme::{self, height, radius, text};
use editor_ui::ui::{self, icon_button};
use gpui::{div, prelude::*, px, AnimationExt, Context, ElementId, Window};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A recent project's row.
const RECENT_ROW: f32 = 52.0;

/// A project opened before.
#[derive(Clone, Debug)]
pub(super) struct Recent {
    pub(super) path: PathBuf,
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
    pub(super) fn quick(path: &Path) -> Self {
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
    pub(super) fn inspected(path: &Path) -> Self {
        let mut recent = Self::quick(path);
        let info = kerf_bridge::project_inspect(path);
        if !info.name.trim().is_empty() {
            recent.name = info.name;
        }
        recent.branch = info.branch;
        recent.presets = info.presets;
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

/// When its document last changed.
fn edited(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path.join("project.kerf"))
        .and_then(|metadata| metadata.modified())
        .ok()
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

impl LauncherView {
    /// The recent project chosen opens, unless its folder is gone.
    pub(super) fn open_chosen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn clear_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.refresh();
        self.prefs.recent.clear();
        self.prefs.save();
        self.recent.clear();
        self.recent_chosen = 0;
        if self
            .stops
            .get(&Stop::ClearRecent)
            .is_some_and(|handle| handle.is_focused(window))
        {
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    /// The recent projects, under their heading; none yet, a line that says
    /// where they come from.
    pub(super) fn recent_list(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let idle = self.busy.is_none();
        let keyed = self
            .handle(Stop::Recent)
            .is_some_and(|handle| handle.is_focused(window));
        let rows = self.recent.iter().enumerate().map(|(index, recent)| {
            self.recent_row(
                index,
                recent,
                keyed && index == self.recent_chosen,
                idle,
                cx,
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
            .child(self.recent_head(window, cx))
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

    /// "Recent", and Clear while there are any.
    fn recent_head(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement + use<> {
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
                            .on_click(
                                cx.listener(|this, _, window, cx| this.clear_recent(window, cx)),
                            ),
                    ),
                )
            })
    }

    /// A recent project's row (`index` among them; `chosen` by Up and Down
    /// while the list has the keys): its folder, name and place, its branch,
    /// when it was edited and its presets; one gone can be forgotten. It
    /// fades up after the rows before it.
    fn recent_row(
        &self,
        index: usize,
        recent: &Recent,
        chosen: bool,
        idle: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let ink = if recent.exists {
            theme::ink()
        } else {
            theme::dis()
        };
        let meta = recent.meta();
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
    }
}
