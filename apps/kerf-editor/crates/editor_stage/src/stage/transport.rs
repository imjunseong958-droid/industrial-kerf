//! The transport under the picture: Go to start, Previous frame, Play,
//! Next frame and Go to end (each its command, its key in its tip, greyed
//! with why while it cannot run), the timecode, Loop, the viewer's zoom and
//! its menu, the mask tools with the hint of the one out, and at its end
//! what the preview needs or plays.

use super::picture::{zoom_words, ZOOMS};
use super::shown::Shown;
use super::tools::MaskTool;
use super::StageView;
use editor_state::model;
use editor_ui::icons::Icon;
use editor_ui::theme::{self, layout, text};
use editor_ui::ui::{self, icon_button};
use gpui::{div, prelude::*, px, App, Context, DismissEvent, Div, ElementId, Window};
use serde_json::Value;
use std::collections::HashMap;

/// The transport's controls: Go to start, Previous frame, Play, Next
/// frame, Go to end; each its command, name and icon.
pub const TRANSPORT: [(&str, &str, Icon); 5] = [
    ("playback.goToStart", "Go to start", Icon::StepBack),
    (
        "playback.previousFrame",
        "Previous frame",
        Icon::ChevronLeft,
    ),
    ("playback.toggle", "Play", Icon::Play),
    ("playback.nextFrame", "Next frame", Icon::ChevronRight),
    ("playback.goToEnd", "Go to end", Icon::StepForward),
];

/// The transport's other commands whose keys its tips say: Loop and the
/// viewer's zoom.
const TRANSPORT_MORE: [&str; 2] = ["playback.loop", "viewer.zoom"];

/// What a transport control's tip says: its name while it can be pressed;
/// greyed, its name and why not.
pub fn transport_tip(name: &str, reason: Option<&str>) -> String {
    match reason.filter(|reason| !reason.is_empty()) {
        Some(reason) => format!("{name} · {reason}"),
        None => name.to_owned(),
    }
}

/// Why each transport control cannot run in `context` (the registry's
/// command context, `Session::command_context`), by its command: no
/// kerfd, or a timeline that plays only once a preview is rendered.
pub fn transport_reasons(context: &Value) -> HashMap<&'static str, String> {
    let states = kerf_bridge::commands_states(context);
    TRANSPORT
        .iter()
        .filter_map(|(id, _, _)| {
            let reason = editor_session::palette::unavailable(&states, id)?;
            Some((*id, reason.to_owned()))
        })
        .collect()
}

impl StageView {
    /// Why each transport control cannot run now (no kerfd; a preview to
    /// render first): asked of the registry again only when what it is
    /// asked about changes, not on every frame played.
    pub(super) fn follow_transport(&mut self, cx: &mut Context<Self>) {
        let mut context = self.session.read(cx).command_context();
        if let Some(context) = context.as_object_mut() {
            context.remove("playheadFrame");
        }
        if self
            .transport_asked
            .as_ref()
            .is_none_or(|(asked, _)| *asked != context)
        {
            let reasons = transport_reasons(&context);
            self.transport_asked = Some((context, reasons));
        }
    }

    /// The transport's keys, read again when the keymap has changed;
    /// whether it had.
    pub(super) fn follow_keymap(&mut self, cx: &App) -> bool {
        let keymap = self.session.read(cx).prefs.keymap.wire();
        let changed = self.keys.0 != keymap;
        if changed {
            let keys = editor_session::commands::catalog(keymap)
                .into_iter()
                .filter(|command| {
                    TRANSPORT.iter().any(|(id, _, _)| *id == command.id)
                        || TRANSPORT_MORE.contains(&command.id.as_str())
                })
                .filter_map(|command| {
                    let key = command
                        .keys
                        .first()
                        .map(|key| editor_state::shortcuts::shown_keys(key))?;
                    Some((command.id, key))
                })
                .collect();
            self.keys = (keymap.to_owned(), keys);
        }
        changed
    }

    /// The transport, under the picture. Its mask tools are greyed while
    /// the clip `shown` over the picture cannot be seen to draw on.
    pub(super) fn transport(
        &self,
        shown: Option<&Shown>,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let keys = self.keys.1.clone();
        let reasons = self
            .transport_asked
            .as_ref()
            .map(|(_, reasons)| reasons.clone())
            .unwrap_or_default();
        let session = self.session.read(cx);
        let fps = session.timeline().rate();
        let position = model::timecode(session.state.position, fps);
        let length = model::timecode(session.state.program_length(), fps);
        let playing = session.state.playing;
        let (mark_in, mark_out, looping) = session.marks();
        let marked = mark_in.is_some() || mark_out.is_some();
        let drawable = shown.is_some_and(|shown| shown.placement.picture.shows());
        let preview_control = self.preview_control(cx);
        div()
            .h(px(layout::TRANSPORT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::space::S4))
            .px(px(theme::space::S12))
            // Go to start, Previous frame, Play, Next frame, Go to end: each
            // its command, its key in its tip, greyed with why while it
            // cannot run.
            .children(transport_buttons(playing, &keys, &reasons))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S6))
                    .pl(px(theme::space::S8))
                    .child(ui::label(position, text::TIMECODE_STRONG, theme::ink()))
                    .child(ui::label("/", text::TIMECODE, theme::ink3()))
                    .child(ui::label(length, text::TIMECODE, theme::ink3())),
            )
            .child(loop_button(looping, marked, &keys, cx))
            .child(self.zoom_button(&keys, cx))
            .child(ui::hairline(false).h(px(16.0)).mx(px(theme::space::S6)))
            .children(self.tool_buttons(drawable, cx))
            .children(self.tool.map(|tool| {
                ui::line(tool.hint(), text::CONTROL, theme::ink3())
                    .pl(px(theme::space::S6))
                    .min_w_0()
                    .flex_shrink()
            }))
            .child(div().flex_1())
            .children(preview_control)
    }

    /// The viewer's zoom ("Fit", "50%"), which opens its menu over it.
    fn zoom_button(&self, keys: &HashMap<String, String>, cx: &Context<Self>) -> Div {
        div()
            .relative()
            .pl(px(theme::space::S4))
            .child(ui::tip_on(
                theme::type_style(
                    ui::capture_trigger("viewer-zoom", {
                        let this = cx.entity().downgrade();
                        move |_, window, cx| {
                            this.update(cx, |this, cx| this.toggle_zoom_menu(window, cx))
                                .ok();
                        }
                    }),
                    text::CONTROL,
                )
                .flex()
                .items_center()
                .h(px(22.0))
                .px(px(7.0))
                .rounded(px(theme::radius::CONTROL))
                .cursor_pointer()
                .text_color(if self.zoom_menu.is_some() || self.zoom > 0.0 {
                    theme::ink()
                } else {
                    theme::ink3()
                })
                .when(self.zoom_menu.is_some(), |button| button.bg(theme::on()))
                .hover(|style| style.bg(theme::raised()).text_color(theme::ink()))
                .child(zoom_words(self.zoom)),
                // Its menu says all the tip would: none while it is open.
                self.zoom_menu.is_none().then(|| {
                    ui::Tip::new("Viewer zoom").maybe_key(keys.get("viewer.zoom").cloned())
                }),
            ))
            .children(self.zoom_menu())
    }

    /// The mask tools, the one out held down; greyed while there is no
    /// picture clip to draw on (`drawable`).
    fn tool_buttons(&self, drawable: bool, cx: &Context<Self>) -> [ui::IconButton; 3] {
        MaskTool::ALL.map(|tool| {
            icon_button(tool.id(), tool.icon())
                .on(self.tool == Some(tool))
                .disabled(!drawable)
                .tooltip(tool.label())
                .key(editor_state::shortcuts::shown_keys(tool.shortcut()))
                .reason(transport_tip(
                    tool.label(),
                    Some("Choose a picture clip to draw on"),
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_tool(tool, window, cx);
                }))
        })
    }

    /// The viewer's zoom: 0 is Fit.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Draws the frame at `zoom` (0 fits it to the picture), and closes the
    /// zoom's menu.
    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.zoom = zoom.max(0.0);
        self.zoom_menu = None;
        cx.notify();
    }

    /// Opens the zoom's menu (Fit, 50%, 100%, 200%) over the transport, or
    /// closes it: View ▸ Viewer Zoom… (`viewer.zoom`, ⇧Z) calls it, and the
    /// transport's zoom. Open, the picture takes Esc to close it.
    pub fn toggle_zoom_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.zoom_menu.take().is_some() {
            cx.notify();
            return;
        }
        let this = cx.entity().downgrade();
        let items = ZOOMS
            .map(|zoom| {
                let this = this.clone();
                let item = ui::MenuItem::new(zoom_words(zoom))
                    .checked((zoom - self.zoom).abs() < f32::EPSILON)
                    .on_choose(move |_, cx| {
                        let _ = this.update(cx, |stage, cx| stage.set_zoom(zoom, cx));
                    });
                if zoom <= 0.0 {
                    item.key(editor_state::shortcuts::shown_keys("Shift+Z"))
                } else {
                    item
                }
            })
            .to_vec();
        // Its first row is lit at once, so the arrows and Return work from
        // the keys (⇧Z) as from the pointer.
        let menu = cx.new(|cx| ui::Menu::new(items, window, cx).width(168.0).from_keys());
        let dismissed = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.zoom_menu = None;
            cx.notify();
        });
        self.zoom_menu = Some((menu, dismissed));
        cx.notify();
    }

    /// The zoom's menu, over the transport's zoom.
    fn zoom_menu(&self) -> Option<impl IntoElement + use<>> {
        let (menu, _) = self.zoom_menu.as_ref()?;
        Some(
            div()
                .absolute()
                .bottom(px(layout::TRANSPORT - 4.0))
                .left(px(0.0))
                .child(menu.clone()),
        )
    }
}

/// Go to start, Previous frame, Play (Pause while `playing`), Next frame
/// and Go to end: each runs its command, its key (`keys`) in its tip,
/// greyed with why while it cannot run (`reasons`).
fn transport_buttons(
    playing: bool,
    keys: &HashMap<String, String>,
    reasons: &HashMap<&'static str, String>,
) -> [ui::IconButton; 5] {
    TRANSPORT.map(|(id, name, icon)| {
        let play = id == "playback.toggle";
        let (name, icon) = match (play, playing) {
            (true, true) => ("Pause", Icon::Pause),
            _ => (name, icon),
        };
        let reason = reasons.get(id).cloned();
        let mut button = icon_button(ElementId::Name(format!("transport-{id}").into()), icon)
            .tooltip(name)
            .disabled(reason.is_some())
            .on_click(move |_, window, cx| {
                window.dispatch_action(
                    Box::new(editor_session::commands::RunCommand { id: id.into() }),
                    cx,
                );
            });
        if play {
            button = button.color(theme::ink());
        }
        if let Some(key) = keys.get(id) {
            button = button.key(key.clone());
        }
        if let Some(reason) = reason {
            button = button.reason(transport_tip(name, Some(&reason)));
        }
        button
    })
}

/// Loop, held down while it is on: "Loop in to out" with marks set.
fn loop_button(
    looping: bool,
    marked: bool,
    keys: &HashMap<String, String>,
    cx: &Context<StageView>,
) -> Div {
    div().pl(px(theme::space::S8)).child(
        icon_button("loop-playback", Icon::Loop)
            .on(looping)
            .tooltip(if looping && marked {
                "Loop in to out"
            } else {
                "Loop"
            })
            .when_some(keys.get("playback.loop").cloned(), |button, key| {
                button.key(key)
            })
            .on_click(cx.listener(|_, _, window, cx| {
                window.dispatch_action(
                    Box::new(editor_session::commands::RunCommand::new("playback.loop")),
                    cx,
                );
            })),
    )
}
