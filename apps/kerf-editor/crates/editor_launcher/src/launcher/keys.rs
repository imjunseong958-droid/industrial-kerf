//! The launcher's keyboard: Tab and Shift-Tab from control to control in
//! the order they show, Up and Down among the recent projects or the
//! presets, Space and Return on the control the keys are on, Left and Right
//! on the format and the frame rate, Return for what the page is for, Esc
//! to put the form away; and the ring the keys show on a control.

use super::details::{github_account, RATES};
use super::form::Format;
use super::{LauncherView, Mode};
use crate::presets::Fit;
use crate::settings::Page;
use editor_ui::input::TextInput;
use editor_ui::theme::{self, radius};
use gpui::{
    div, prelude::*, px, App, Context, ElementId, Entity, FocusHandle, Focusable, KeyBinding,
    Keystroke, Window,
};

/// What Esc does in the launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherEscape {
    /// The form open goes, and the keys are on the launcher again.
    PutFormAway,
    /// The launcher is given up and the editor behind it comes back.
    GiveBack,
    /// Nothing: a form whose task runs stays, and the launcher the editor
    /// started with (no editor behind it) stays.
    Nothing,
}

/// What Esc does with a form open or not, a task of the form running or
/// not, and an editor behind the launcher (New Project… from it) or not.
pub fn launcher_escape(form_open: bool, busy: bool, over_editor: bool) -> LauncherEscape {
    match (form_open, busy, over_editor) {
        (true, false, _) => LauncherEscape::PutFormAway,
        (true, true, _) => LauncherEscape::Nothing,
        (false, _, true) => LauncherEscape::GiveBack,
        (false, _, false) => LauncherEscape::Nothing,
    }
}

gpui::actions!(
    launcher,
    [
        NextControl,
        PreviousControl,
        Press,
        Choose,
        Back,
        ChosenUp,
        ChosenDown,
        Earlier,
        Later
    ]
);

/// The launcher's key context: Tab, the arrows, Return and Esc.
pub(super) const CONTEXT: &str = "Launcher";
/// A control's own key context, where Space and Return press it.
pub(super) const CONTROL: &str = "LauncherControl";

/// The ring round the control the keyboard is on.
const RING: f32 = 2.0;

/// A text field of the launcher's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Field {
    Name,
    Location,
    Url,
    Remote,
    Start(usize),
    Text(usize),
}

/// What the keyboard can be on, in the order Tab goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Stop {
    Settings,
    /// Cancel, with an editor behind the launcher.
    Cancel,
    /// A way to begin; None opens a folder.
    Way(Option<Mode>),
    Field(Field),
    ChooseLocation,
    Format,
    Details,
    Rate,
    Git,
    GitHub,
    /// Create Project, or Clone.
    Act,
    PresetList,
    PresetFile,
    SlotFile(usize),
    Recent,
    ClearRecent,
}

impl Stop {
    /// The stops there from the start, each with its focus for as long as
    /// the launcher is: all but the fields (whose focus is their own) and a
    /// preset's slots (theirs come with the slots).
    pub(super) const FIXED: [Stop; 17] = [
        Stop::Settings,
        Stop::Cancel,
        Stop::Way(Some(Mode::New)),
        Stop::Way(Some(Mode::Preset)),
        Stop::Way(None),
        Stop::Way(Some(Mode::Clone)),
        Stop::ChooseLocation,
        Stop::Format,
        Stop::Details,
        Stop::Rate,
        Stop::Git,
        Stop::GitHub,
        Stop::Act,
        Stop::PresetList,
        Stop::PresetFile,
        Stop::Recent,
        Stop::ClearRecent,
    ];
}

/// Binds the launcher's keys, unless they are bound. Binding a keymap
/// clears them, so this runs again when Settings closes.
pub(super) fn bind_keys(cx: &mut App) {
    let bound = Keystroke::parse("tab").is_ok_and(|tab| {
        cx.all_bindings_for_input(&[tab])
            .iter()
            .any(|binding| binding.action().partial_eq(&NextControl))
    });
    if bound {
        return;
    }
    cx.bind_keys([
        KeyBinding::new("tab", NextControl, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PreviousControl, Some(CONTEXT)),
        KeyBinding::new("enter", Choose, Some(CONTEXT)),
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("up", ChosenUp, Some(CONTEXT)),
        KeyBinding::new("down", ChosenDown, Some(CONTEXT)),
        KeyBinding::new("enter", Press, Some(CONTROL)),
        KeyBinding::new("space", Press, Some(CONTROL)),
        KeyBinding::new("left", Earlier, Some(CONTROL)),
        KeyBinding::new("right", Later, Some(CONTROL)),
    ]);
}

impl LauncherView {
    /// The controls on screen, in the order Tab goes.
    fn stop_order(&self) -> Vec<Stop> {
        let mut order = vec![Stop::Settings];
        if self.over_editor {
            order.push(Stop::Cancel);
        }
        order.extend([
            Stop::Way(Some(Mode::New)),
            Stop::Way(Some(Mode::Preset)),
            Stop::Way(None),
            Stop::Way(Some(Mode::Clone)),
        ]);
        let details = |order: &mut Vec<Stop>| {
            order.extend([
                Stop::Field(Field::Name),
                Stop::Field(Field::Location),
                Stop::ChooseLocation,
            ]);
        };
        // Only what can be used: a switch with nothing to share with, or
        // the remote while GitHub is chosen, is passed over.
        let git_found = self.git_found();
        let signed_in = self.tools.as_ref().and_then(github_account).is_some();
        let github = self.github;
        let sharing = |order: &mut Vec<Stop>, open: bool| {
            order.push(Stop::Details);
            if open {
                order.push(Stop::Rate);
                if git_found {
                    order.push(Stop::Git);
                }
                if signed_in {
                    order.push(Stop::GitHub);
                }
                if git_found && !github {
                    order.push(Stop::Field(Field::Remote));
                }
            }
        };
        match self.mode {
            Some(Mode::New) => {
                details(&mut order);
                order.push(Stop::Format);
                sharing(&mut order, self.details);
                order.push(Stop::Act);
            }
            Some(Mode::Preset) => {
                if !self.preset.choices.is_empty() {
                    order.push(Stop::PresetList);
                }
                order.push(Stop::PresetFile);
                for (index, slot) in self.preset.slots.iter().enumerate() {
                    if slot.fit.as_ref().is_some_and(Fit::start_open) {
                        order.push(Stop::Field(Field::Start(index)));
                    }
                    order.push(Stop::SlotFile(index));
                }
                order.extend(
                    (0..self.preset.texts.len()).map(|index| Stop::Field(Field::Text(index))),
                );
                details(&mut order);
                sharing(&mut order, self.details);
                order.push(Stop::Act);
            }
            Some(Mode::Clone) => {
                order.extend([
                    Stop::Field(Field::Url),
                    Stop::Field(Field::Location),
                    Stop::ChooseLocation,
                    Stop::Act,
                ]);
            }
            None => {}
        }
        if !self.recent.is_empty() {
            order.extend([Stop::Recent, Stop::ClearRecent]);
        }
        order
    }

    fn field_input(&self, field: Field) -> Option<&Entity<TextInput>> {
        match field {
            Field::Name => Some(&self.name),
            Field::Location => Some(&self.location),
            Field::Url => Some(&self.url),
            Field::Remote => Some(&self.remote),
            Field::Start(index) => self.preset.slots.get(index).map(|slot| &slot.start),
            Field::Text(index) => self.preset.texts.get(index).map(|text| &text.input),
        }
    }

    /// Lays out the order Tab goes in, each control on screen with its
    /// focus, once each time the launcher is drawn: what the keys and the
    /// rings go by until it is drawn again.
    pub(super) fn arrange(&mut self, cx: &App) {
        let order = self
            .stop_order()
            .into_iter()
            .filter_map(|stop| {
                let handle = match stop {
                    Stop::Field(field) => self.field_input(field)?.read(cx).focus_handle(cx),
                    stop => self.stops.get(&stop)?.clone(),
                };
                Some((stop, handle))
            })
            .collect();
        self.order = order;
    }

    fn focused(&self, window: &Window) -> Option<Stop> {
        self.order
            .iter()
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(stop, _)| *stop)
    }

    /// The focus of the control `stop` while it is one of the controls on
    /// screen, as last drawn.
    pub(super) fn handle(&self, stop: Stop) -> Option<FocusHandle> {
        self.order
            .iter()
            .find(|(known, _)| *known == stop)
            .map(|(_, handle)| handle.clone())
    }

    /// Tab and Shift-Tab: the next control, or the one before, round.
    fn step_focus(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let order = &self.order;
        if order.is_empty() {
            return;
        }
        let count = order.len() as isize;
        let next = match order
            .iter()
            .position(|(_, handle)| handle.is_focused(window))
        {
            Some(at) => (at as isize + by).rem_euclid(count),
            None if by > 0 => 0,
            None => count - 1,
        };
        let (_, handle) = &order[next as usize];
        window.focus(handle);
        cx.notify();
    }

    /// Up and Down: the recent project chosen, or the preset.
    fn step_chosen(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused(window) {
            Some(Stop::PresetList) if !self.preset.choices.is_empty() => {
                let last = self.preset.choices.len() as isize - 1;
                let at = self
                    .preset
                    .chosen
                    .map_or(0, |at| (at as isize + by).clamp(0, last) as usize);
                self.load_choice(at, window, cx);
            }
            Some(Stop::Recent) if !self.recent.is_empty() => {
                let last = self.recent.len() as isize - 1;
                self.recent_chosen = (self.recent_chosen as isize + by).clamp(0, last) as usize;
                cx.notify();
            }
            // Coming to the list, the arrow shows the project chosen first.
            Some(Stop::ClearRecent) | None if !self.recent.is_empty() => {
                if let Some(handle) = self.handle(Stop::Recent) {
                    window.focus(&handle);
                }
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    /// Space or Return on a control: what pressing it does.
    fn press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stop) = self.focused(window) else {
            return;
        };
        match stop {
            Stop::Settings => self.show_settings(Page::Chat, window, cx),
            Stop::Cancel => self.cancel(cx),
            // A way is pressed by its own click, which GPUI gives a focused
            // element on Space and Return.
            Stop::Way(_) => {}
            Stop::ChooseLocation => self.choose_location(window, cx),
            Stop::Format | Stop::Rate => self.cycle(stop, 1, cx),
            Stop::Details => self.set_details(!self.details, cx),
            Stop::Git => self.set_git(!self.git, cx),
            Stop::GitHub => self.set_github(!self.github, cx),
            Stop::Act => self.choose(window, cx),
            Stop::PresetList => {
                let at = self.preset.chosen.unwrap_or(0);
                self.load_choice(at, window, cx);
            }
            Stop::PresetFile => self.choose_preset_file(window, cx),
            Stop::SlotFile(slot) => self.choose_slot_file(slot, window, cx),
            Stop::Recent => self.open_chosen(window, cx),
            Stop::ClearRecent => self.clear_recent(window, cx),
            Stop::Field(_) => {}
        }
    }

    /// Left and Right on the template or the frame rate.
    fn cycle(&mut self, stop: Stop, by: isize, cx: &mut Context<Self>) {
        let step = |at: usize, count: usize| (at as isize + by).rem_euclid(count as isize) as usize;
        match stop {
            Stop::Format => {
                let at = Format::ALL
                    .iter()
                    .position(|f| *f == self.format)
                    .unwrap_or(0);
                self.format = Format::ALL[step(at, Format::ALL.len())];
            }
            Stop::Rate => {
                let at = RATES.iter().position(|(r, _)| *r == self.rate).unwrap_or(0);
                self.rate = RATES[step(at, RATES.len())].0;
            }
            _ => return,
        }
        cx.notify();
    }

    fn arrow(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused(window) {
            Some(stop @ (Stop::Format | Stop::Rate)) => self.cycle(stop, by, cx),
            _ => cx.propagate(),
        }
    }

    /// `control` with the ring the keyboard shows on it, taking Space and
    /// Return when it is on it.
    pub(super) fn ring(
        &self,
        stop: Stop,
        window: &Window,
        control: impl IntoElement,
    ) -> gpui::Stateful<gpui::Div> {
        let handle = self.handle(stop);
        let focused = handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        let mut ring = div()
            .id(ElementId::Name(format!("launcher-ring-{stop:?}").into()))
            .key_context(CONTROL)
            .flex_none()
            .p(px(RING))
            .rounded(px(radius::CONTROL + RING))
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::with_alpha(theme::accent(), 0.0)
            })
            .child(control);
        if let Some(handle) = &handle {
            ring = ring.track_focus(handle);
        }
        ring
    }

    /// `column` under the launcher's own keys, which answer only there, not
    /// in Settings over it.
    pub(super) fn with_keys(&self, column: impl IntoElement, cx: &Context<Self>) -> gpui::Div {
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &NextControl, window, cx| this.step_focus(1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PreviousControl, window, cx| {
                    this.step_focus(-1, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Choose, window, cx| this.choose(window, cx)))
            .on_action(cx.listener(|this, _: &Back, window, cx| this.back(window, cx)))
            .on_action(
                cx.listener(|this, _: &ChosenUp, window, cx| this.step_chosen(-1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ChosenDown, window, cx| this.step_chosen(1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &Press, window, cx| this.press(window, cx)))
            .on_action(cx.listener(|this, _: &Earlier, window, cx| this.arrow(-1, window, cx)))
            .on_action(cx.listener(|this, _: &Later, window, cx| this.arrow(1, window, cx)))
            .size_full()
            .child(column)
    }
}
