//! The ways to begin (New Project, From Preset, Open Folder…, Clone from
//! GitHub) and the form a way chosen fades in under them: its rows, a line
//! that says why it cannot be made yet or where it goes, and Create
//! Project or Clone; and the pieces its rows are built of.

use super::keys::{Stop, CONTROL};
use super::plan::{
    clone_problem, details_problem, expanded, folder_name, home_relative, repository_name,
};
use super::{LauncherView, Mode};
use crate::presets::{self, Picked};
use editor_ui::icons::Icon;
use editor_ui::input::TextInput;
use editor_ui::motion;
use editor_ui::theme::{self, height, radius, text};
use editor_ui::ui;
use gpui::{
    div, prelude::*, px, AnimationExt, AnyElement, App, Context, ElementId, Entity, Focusable,
    MouseButton, PathPromptOptions, Window,
};
use std::path::PathBuf;

/// A new project's frame: kerf-collab's templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Standard,
    Vertical,
    Square,
}

impl Format {
    pub(super) const ALL: [Format; 3] = [Format::Standard, Format::Vertical, Format::Square];

    pub(super) fn label(self) -> &'static str {
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

    pub(super) fn wire(self) -> &'static str {
        match self {
            Format::Standard => "standard",
            Format::Vertical => "vertical",
            Format::Square => "square",
        }
    }
}

/// A form's labels stand in a column this wide.
pub(super) const FORM_LABEL: f32 = 96.0;

/// A form's row: what it asks, and the control that answers.
pub(super) fn form_row(label: &'static str, control: impl IntoElement) -> gpui::AnyElement {
    form_row_owned(label.to_owned(), control)
}

/// A form's row whose words are made as it is drawn.
pub(super) fn form_row_owned(label: String, control: impl IntoElement) -> gpui::AnyElement {
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
pub fn field(input: &Entity<TextInput>, window: &Window, cx: &App) -> gpui::Div {
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

impl LauncherView {
    /// A way to begin: pressed, its form fades in under the ways, or goes;
    /// Open Folder… (no `mode`) asks for a folder.
    fn way(
        &self,
        mode: Option<Mode>,
        icon: Icon,
        title: &'static str,
        hint: &'static str,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
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

    /// The ways to begin, side by side.
    pub(super) fn ways(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .mt(px(28.0))
            .flex()
            .gap(px(theme::space::S12))
            .child(self.way(
                Some(Mode::New),
                Icon::Plus,
                "New Project",
                "An empty project, in a folder of its own",
                window,
                cx,
            ))
            .child(self.way(
                Some(Mode::Preset),
                Icon::Sparkle,
                "From Preset",
                "A shared preset, filled with your media",
                window,
                cx,
            ))
            .child(self.way(
                None,
                Icon::Folder,
                "Open Folder…",
                "A project, or a folder of footage to start one from",
                window,
                cx,
            ))
            .child(self.way(
                Some(Mode::Clone),
                Icon::Import,
                "Clone from GitHub",
                "A shared project, from its repository",
                window,
                cx,
            ))
    }

    /// A way chosen: its form, the keys in its first field (on From
    /// Preset, on the way itself); chosen again, the form goes.
    pub(super) fn choose_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        match self.mode {
            Some(Mode::New) | Some(Mode::Preset) => self.create(cx),
            Some(Mode::Clone) => self.clone_repository(cx),
            None => self.open_chosen(window, cx),
        }
    }

    /// Where the project or the clone goes, and Choose… beside it.
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

    /// Choose…: a folder for the project or the clone, chosen in the system's
    /// chooser.
    pub(super) fn choose_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn location(&self, cx: &App) -> PathBuf {
        expanded(self.location.read(cx).text().trim())
    }

    /// Why Return cannot do what the page is for, as it is typed; None
    /// when it can.
    pub(super) fn form_problem(&self, cx: &App) -> Option<String> {
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

    /// The form of the way chosen (`mode`): its rows, then why it cannot be
    /// made yet as it is typed, else where it goes, beside Create Project or
    /// Clone.
    pub(super) fn form(
        &self,
        mode: Mode,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let idle = self.busy.is_none();
        let problem = if idle { self.form_problem(cx) } else { None };
        let location = self.location(cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        let (destination, act_id, act_words): (String, &'static str, &'static str) = match mode {
            Mode::New | Mode::Preset => {
                if mode == Mode::Preset {
                    rows.extend(self.preset_rows(window, cx));
                }
                let folder = folder_name(self.name.read(cx).text());
                rows.push(form_row("Name", field(&self.name, window, cx).flex_1()));
                rows.push(form_row("Location", self.location_row(window, cx)));
                if mode == Mode::New {
                    rows.push(self.format_row(window, cx));
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
        let act = self.act(act_id, act_words, problem, idle, window, cx);
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

    /// New Project's format, and the size it makes.
    fn format_row(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let formats = Format::ALL.map(|format| (format, format.label()));
        form_row(
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
        )
    }

    /// Create Project or Clone (`act_id`, `act_words`), with the ring the
    /// keys show on it: greyed while a task runs (`idle`) or while what is
    /// typed cannot be made (`problem`), its tip saying why.
    fn act(
        &self,
        act_id: &'static str,
        act_words: &'static str,
        problem: Option<String>,
        idle: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        // Greyed by what is typed, its tip says why.
        let act_button = ui::button(act_id, act_words)
            .primary()
            .disabled(!idle || problem.is_some())
            .on_click(cx.listener(|this, _, window, cx| this.choose(window, cx)));
        let act_button = match problem {
            Some(why) => act_button.reason(why),
            None => act_button,
        };
        div()
            .id("launcher-act-why")
            .child(self.ring(Stop::Act, window, act_button))
    }

    /// A segmented control's or a switch's choice, carried to `this`.
    pub(super) fn chooser<T, F>(
        cx: &Context<Self>,
        choose: F,
    ) -> impl Fn(T, &mut Window, &mut App) + 'static + use<T, F>
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
