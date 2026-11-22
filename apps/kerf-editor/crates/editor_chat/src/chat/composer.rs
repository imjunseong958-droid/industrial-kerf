//! The composer ([`ChatView::composer`]): Kerf's next steps and what the
//! last check found failing over it, each filling it with its request;
//! what waits to be asked ("Next: …"); the box; and under the words its
//! chips (the folder, the playhead's time when it is not sent, the model,
//! what Kerf does without asking), the context meter, and Send or Stop.
//! Where it was laid out is kept for what leaves from over it
//! ([`over_composer`]).

use super::menus::{
    fade_in, menu_frame, next_permissions, permission_look, permission_words, Menu,
};
use super::{ChatView, CyclePermissions, COMPOSER_CONTEXT};
use editor_session::commands::RunCommand;
use editor_state::model;
use editor_state::prefs::{model_label, Provider};
use editor_state::store::{ContextLevel, ContextUse};
use editor_ui::icons::Icon;
use editor_ui::input::InputEvent;
use editor_ui::motion;
use editor_ui::theme::{self, layout, motion as scene, text};
use editor_ui::ui::{self, icon_button};
use gpui::{
    anchored, deferred, div, point, prelude::*, px, AnimationExt, AnyElement, App, Context, Corner,
    Edges, ElementId, Focusable, Rgba, Window,
};
use kerf_bridge::event::CheckSummary;

/// The most the composer's text grows to before it scrolls, in points.
pub const COMPOSER_MOST: f32 = 110.0;

/// The widest the composer's folder chip grows before its name is cut.
pub use editor_ui::theme::layout::COMPOSER_FOLDER_MOST as FOLDER_CHIP_MOST;

/// The room over the composer a flight leaves from, and how far above the
/// composer's top it starts.
pub use editor_ui::theme::layout::{OVER_COMPOSER, OVER_COMPOSER_RISE};

/// What a finished check's summary (`check {status: "done", summary:
/// {rows}}`) offers to ask: "Fix the true peak" for each failing row with a
/// name, with its timecode where it has one ("Fix the true peak at
/// 00:00:04:12"): the words shown, and the request they fill the composer
/// with. A time that is not finite is none, as JSON's `null`.
pub fn check_suggestions(summary: &CheckSummary, fps: f64) -> Vec<(String, String)> {
    summary
        .rows
        .iter()
        .filter(|row| !row.passed)
        .filter_map(|row| {
            let name = row.name.trim();
            let mut chars = name.chars();
            let first = chars.next()?;
            let lowered: String = first.to_lowercase().chain(chars).collect();
            let label = format!("Fix the {lowered}");
            let request = match row.at.filter(|at| at.is_finite()) {
                Some(at) => format!("{label} at {}", model::timecode(at, fps)),
                None => label.clone(),
            };
            Some((label, request))
        })
        .collect()
}

/// Where what comes from the conversation with no tile of its own leaves
/// from, over a composer laid out at `composer`: a poster's room,
/// `OVER_COMPOSER` big, centred over it and `OVER_COMPOSER_RISE` above its
/// top.
pub fn over_composer(composer: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    let (width, height) = OVER_COMPOSER;
    gpui::Bounds::new(
        point(
            composer.center().x - px(width / 2.0),
            composer.top() - px(OVER_COMPOSER_RISE),
        ),
        gpui::size(px(width), px(height)),
    )
}

/// What the composer says, read once for its parts.
struct ComposerFacts {
    busy: bool,
    empty: bool,
    /// Why nothing can be asked now, if so: said over the words and Send.
    refused: Option<String>,
    /// What to ask next: Kerf's next steps, then what the last check found
    /// failing; the words shown, and the request each fills the box with.
    next: Vec<(String, String)>,
    /// Run QC, offered while the box waits empty for words.
    offer_qc: bool,
    /// The live time, struck through, while the stamp is taken off.
    unstamped: Option<String>,
    /// The folder chip's words and tip, the model chip's, and the mode
    /// chip's, with its icon and tone.
    folder_words: String,
    folder_tip: String,
    model_words: String,
    model_tip: String,
    mode_words: &'static str,
    mode_icon: Icon,
    mode_tone: Option<Rgba>,
    mode_tip: ui::Tip,
    context: Option<ContextUse>,
    /// What waits for Kerf to finish.
    queued: Option<String>,
    stopping: bool,
}

impl ChatView {
    pub fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.read(cx).focus(window);
    }

    /// A suggestion fills the composer, to be changed before it is sent.
    /// Puts `words` in the composer and gives it the keys, to be sent or
    /// changed first (a suggestion, Finish's and Export's Ask Kerf).
    pub fn fill(&mut self, words: String, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |composer, cx| composer.set_text(words, cx));
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// The box's own keys: Return sends; Esc stops Kerf while it works,
    /// else leaves the box.
    pub(super) fn hear_composer(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Submit => self.send(window, cx),
            // Esc stops Kerf while it works; else leaves the box and
            // puts away what the window has open (the Media band, a
            // preview rendering), as it would outside the box.
            InputEvent::Cancel => {
                if self.session.read(cx).state.work.busy() {
                    self.stop(cx);
                } else {
                    // From the box, so it reaches the window, before
                    // the box lets the keys go.
                    window.dispatch_action(Box::new(editor_session::commands::Escape), cx);
                    window.blur();
                }
            }
            InputEvent::Changed => cx.notify(),
        }
    }

    /// The composer: what to ask next over it (unless Kerf works), what
    /// waits to be asked, and the box with its chips.
    pub(super) fn composer(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut facts = self.composer_facts(window, cx);
        let next = std::mem::take(&mut facts.next);
        let queued = facts.queued.take();
        let offer_qc = facts.offer_qc;
        div()
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .when((!next.is_empty() || offer_qc) && !facts.busy, |column| {
                column.child(suggestions(next, offer_qc, cx))
            })
            // What waits for Kerf to finish: "Next: …" and Cancel, over the box.
            .when_some(queued, |column, queued| {
                column.child(queued_row(&queued, cx))
            })
            .child(self.composer_box(facts, cx))
            .into_any_element()
    }

    /// What the composer says now: whether Kerf works and the box is empty
    /// or has the keys, why nothing can be asked, what to ask next, and
    /// what its chips say.
    fn composer_facts(&self, window: &Window, cx: &App) -> ComposerFacts {
        let session = self.session.read(cx);
        let busy = session.state.work.busy();
        let empty = self.composer.read(cx).text().trim().is_empty();
        let typing_in = self.composer.read(cx).focus_handle(cx).is_focused(window);
        let refused = self.cannot_ask().map(|refused| refused.reason.clone());
        let provider = session.prefs.provider;
        let model = session.prefs.model().to_owned();
        let permissions = session.prefs.permissions;
        let context = session.state.context_shown().cloned();
        // What to ask next: Kerf's next steps, then what the last check found
        // failing; each fills the composer to be changed before it is sent.
        let mut next: Vec<(String, String)> = session
            .state
            .next_steps
            .iter()
            .map(|step| (step.label.clone(), step.request.clone()))
            .collect();
        next.extend(self.fixes.iter().cloned());
        // Run QC is offered while the box waits empty for words.
        let offer_qc = typing_in && empty && refused.is_none();
        // With the stamp taken off (and a picture to stamp), its chip says so
        // with the live time, struck through; pressed, the time goes again.
        let unstamped = (!session.prefs.send_playhead && session.picture_open != Some(false))
            .then(|| model::timecode(session.state.position, session.timeline().rate()));
        let model_words = if model.is_empty() {
            provider.label().to_owned()
        } else {
            format!("{} · {}", provider.label(), model_label(&model))
        };
        // Where there is a choice it is named as one.
        let model_tip = if provider == Provider::ClaudeCode {
            format!(
                "Model · {}",
                model_label(if model.is_empty() { "default" } else { &model })
            )
        } else {
            format!("Answers come from {model_words}")
        };
        // Where a request goes: the open folder, or the way to one.
        let (folder_words, folder_tip) = match session.project.as_deref() {
            Some(project) => (
                project
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| project.display().to_string()),
                project.display().to_string(),
            ),
            None => ("No folder".to_owned(), "Open a folder".to_owned()),
        };
        let (mode_words, mode_line) = permission_words(permissions);
        let (mode_icon, mode_tone) = permission_look(permissions);
        let mode_tip = ui::Tip::new(format!("{mode_words} · {mode_line}")).key("⇧Tab");
        ComposerFacts {
            busy,
            empty,
            refused,
            next,
            offer_qc,
            unstamped,
            folder_words,
            folder_tip,
            model_words,
            model_tip,
            mode_words,
            mode_icon,
            mode_tone,
            mode_tip,
            context,
            queued: self.queued.text().map(str::to_owned),
            stopping: self.stopping,
        }
    }

    /// The box: Shift-Tab in it goes round what Kerf does without asking;
    /// the words, and its chips under them.
    fn composer_box(&self, facts: ComposerFacts, cx: &Context<Self>) -> gpui::Div {
        div()
            .key_context(COMPOSER_CONTEXT)
            .on_action(cx.listener(|this, _: &CyclePermissions, _, cx| {
                let next = next_permissions(this.session.read(cx).prefs.permissions);
                this.set_permissions(next, cx);
            }))
            .flex()
            .flex_col()
            .gap(px(theme::space::S8))
            .p(px(theme::space::S12))
            .rounded(px(theme::radius::CARD + 4.0))
            .bg(theme::raised())
            .border_1()
            .border_color(theme::line2())
            .child(self.composer_field(facts.refused.clone()))
            .child(self.composer_chips(facts, cx))
    }

    /// The words being written; refused, a hover over them says why.
    fn composer_field(&self, refused: Option<String>) -> AnyElement {
        // Refused, a hover over the words says why.
        let field = div()
            .id("composer-field")
            .w_full()
            .child(self.composer.clone());
        match refused {
            Some(reason) => {
                ui::tipped("composer-field-tip", ui::Tip::new(reason), field).into_any_element()
            }
            None => field.into_any_element(),
        }
    }

    /// The box's row of chips: the folder, the stamp taken off, the model,
    /// what Kerf does without asking; then the context meter, and Send or
    /// Stop.
    fn composer_chips(&self, facts: ComposerFacts, cx: &Context<Self>) -> gpui::Div {
        let ComposerFacts {
            busy,
            empty,
            refused,
            unstamped,
            folder_words,
            folder_tip,
            model_words,
            model_tip,
            mode_words,
            mode_icon,
            mode_tone,
            mode_tip,
            context,
            stopping,
            ..
        } = facts;
        div()
            .flex()
            .items_center()
            .gap(px(theme::space::S6))
            .child(self.menu_chip(
                Menu::Folder,
                "folder-chip",
                (Some(Icon::Folder), folder_words, ui::Tip::new(folder_tip)),
                None,
                cx,
            ))
            .when_some(unstamped, |row, timecode| {
                row.child(stamp_chip(timecode, cx))
            })
            .child(self.menu_chip(
                Menu::Model,
                "model-chip",
                (None, model_words, ui::Tip::new(model_tip)),
                None,
                cx,
            ))
            .child(self.menu_chip(
                Menu::Permissions,
                "permissions-chip",
                (Some(mode_icon), mode_words.to_owned(), mode_tip),
                mode_tone,
                cx,
            ))
            .child(div().flex_1())
            .when_some(context, |row, context| {
                row.child(self.context_meter(context, cx))
            })
            .child(send_or_stop(busy, stopping, empty, refused, cx))
    }

    /// A chip in the composer's row that opens its menu over it: its icon
    /// (if any) and words, and what a hover over it says.
    fn menu_chip(
        &self,
        menu: Menu,
        id: &'static str,
        (icon, words, tip): (Option<Icon>, String, ui::Tip),
        tone: Option<Rgba>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let open = self.menu.as_ref() == Some(&menu);
        let toggle = menu.clone();
        // A chip with a tone of its own (the permission mode's) keeps it,
        // open or not, its icon too, so a mode where Kerf does more on its
        // own always stands out.
        let ink = match tone {
            Some(tone) => tone,
            None if open => theme::ink(),
            None => theme::ink3(),
        };
        // Where the row is short of room the model's words are cut
        // ("OpenAI · fake-mo…"); the folder's are cut only past
        // `FOLDER_CHIP_MOST`, and the mode is said whole.
        let shrink = if menu == Menu::Model { 1.0 } else { 0.0 };
        let chip =
            theme::type_style(div(), text::META)
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(theme::space::S4))
                .h(px(20.0))
                .pl(px(theme::space::S8))
                .rounded(px(10.0))
                .bg(theme::raised())
                .text_color(ink)
                .when_some(icon, |chip, icon| {
                    chip.child(
                        div()
                            .flex_none()
                            .child(icon.view(12.0, tone.unwrap_or(theme::ink3()))),
                    )
                })
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(ui::cut_line(words)),
                )
                .id(id)
                .pr(px(theme::space::S4))
                .cursor_pointer()
                .hover(move |chip| {
                    let chip = chip.bg(theme::hover2());
                    if tone.is_some() {
                        chip
                    } else {
                        chip.text_color(theme::ink2())
                    }
                })
                .child(
                    div()
                        .flex_none()
                        .child(Icon::ChevronDown.view(12.0, theme::ink3())),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_menu(toggle.clone(), window, cx)
                }));
        // Its menu says all the tip would, so the tip waits while it is open.
        let chip = ui::tip_on(chip, (!open).then_some(tip));
        let mut wrapper = div()
            .relative()
            .min_w_0()
            .when(menu == Menu::Folder, |wrapper| {
                wrapper.max_w(px(FOLDER_CHIP_MOST))
            })
            .child(chip);
        wrapper.style().flex_shrink = Some(shrink);
        if let Some(body) = self.kit_menu.clone().filter(|_| open) {
            wrapper = wrapper.child(
                // Over the chip, its left edge on the chip's, over everything.
                div().absolute().bottom_full().left_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomLeft)
                            .offset(point(px(0.0), px(-theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(body),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    /// How full Kerf's context is, beside Send: its ring, and pressed, its
    /// card ("Context 34%", a bar, what that is of) over it.
    fn context_meter(&self, context: ContextUse, cx: &Context<Self>) -> AnyElement {
        let open = self.menu.as_ref() == Some(&Menu::Context);
        let color = context_color(context.level());
        let fraction = context.fraction() as f32;
        let meter = div()
            .id("context-meter")
            .size(px(24.0))
            .flex_none()
            .rounded(px(theme::radius::CONTROL))
            .cursor_pointer()
            .hover(|meter| meter.bg(theme::hover2()))
            .child(context_ring(fraction, color))
            .on_click(
                cx.listener(|this, _, window, cx| this.toggle_menu(Menu::Context, window, cx)),
            );
        let meter = if open {
            meter.into_any_element()
        } else {
            ui::tipped("context-meter-tip", ui::Tip::new(context.describe()), meter)
                .into_any_element()
        };
        let mut wrapper = div().relative().flex_none().child(meter);
        if open {
            let card = menu_frame(px(260.0), cx)
                .p(px(theme::space::S12))
                .gap(px(theme::space::S8))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(ui::label("Context", text::MENU_ITEM, theme::ink()).flex_1())
                        .child(ui::label(
                            format!("{}%", context.percent()),
                            text::MENU_QUIET,
                            theme::ink3(),
                        )),
                )
                .child(
                    div()
                        .h(px(4.0))
                        .w_full()
                        .rounded_full()
                        .bg(theme::track())
                        .child(
                            div()
                                .h_full()
                                .min_w(px(4.0))
                                .w(gpui::relative(fraction))
                                .rounded_full()
                                .bg(color),
                        ),
                )
                .child(
                    theme::type_style(div(), text::MENU_QUIET)
                        .text_color(theme::ink3())
                        .child(context.note()),
                );
            wrapper = wrapper.child(
                div().absolute().bottom_full().right_0().child(
                    deferred(
                        anchored()
                            .anchor(Corner::BottomRight)
                            .offset(point(px(0.0), px(-theme::space::S4)))
                            .snap_to_window_with_margin(Edges::all(px(theme::space::S8)))
                            .child(fade_in(card, "context-card".to_owned())),
                    )
                    .with_priority(1),
                ),
            );
        }
        wrapper.into_any_element()
    }

    /// The composer at the foot of the conversation's column, and where it
    /// was laid out, for what comes from over it.
    pub(super) fn composer_foot(&self, composer: AnyElement) -> impl IntoElement + use<> {
        div()
            .flex_none()
            .w_full()
            .flex()
            .justify_center()
            .px(px(24.0))
            .pb(px(theme::space::S16))
            .child(
                div()
                    .relative()
                    .w_full()
                    .min_w_0()
                    .max_w(px(layout::CONVERSATION_COLUMN))
                    .child(composer)
                    // Where it is, for what comes from over it.
                    .child({
                        let at = self.composer_at.clone();
                        gpui::canvas(move |bounds, _, _| at.set(Some(bounds)), |_, _, _, _| {})
                            .absolute()
                            .inset_0()
                    }),
            )
    }

    /// Where what comes from the conversation with no tile of its own
    /// leaves from: a poster's room over the composer's middle.
    pub fn over_composer(&self) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.composer_at.get().map(over_composer)
    }

    /// The top of the composer with what stands over it (Kerf's next steps,
    /// Run QC, what waits to be asked), as last laid out: the window's
    /// notices sit above it rather than over them.
    pub fn composer_top(&self) -> Option<f32> {
        self.composer_at
            .get()
            .map(|bounds| f32::from(bounds.origin.y))
    }
}

/// What to ask next, over the box (`next`: the words shown, and the
/// request each fills it with), and Run QC while the box has the keys and
/// is empty (`offer_qc`).
fn suggestions(next: Vec<(String, String)>, offer_qc: bool, cx: &Context<ChatView>) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .gap(px(theme::space::S6))
        // A press here leaves the keys in the box: Run QC,
        // offered only while the box has them, stays under
        // the pointer to be let go on, and the row does not
        // shift as it goes.
        .on_any_mouse_down(|_, window, _| window.prevent_default())
        .children(
            next.into_iter()
                .enumerate()
                .map(|(index, (words, request))| {
                    ui::button(ElementId::Name(format!("next-{index}").into()), words)
                        .small()
                        .icon(Icon::Sparkle)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.fill(request.clone(), window, cx)
                        }))
                }),
        )
        .when(offer_qc, |row| {
            row.child(
                ui::button("run-qc", "Run QC on the whole sequence")
                    .small()
                    .icon(Icon::Check)
                    .on_click(cx.listener(|_, _, window, cx| {
                        window.dispatch_action(Box::new(RunCommand::new("qc.run")), cx)
                    })),
            )
        })
}

/// What waits for Kerf to finish (`queued`), over the box: "Next: …" and
/// Cancel.
fn queued_row(queued: &str, cx: &Context<ChatView>) -> impl IntoElement + use<> {
    let line = queued.split_whitespace().collect::<Vec<_>>().join(" ");
    div()
        .flex()
        .items_center()
        .gap(px(theme::space::S8))
        .pl(px(theme::space::S4))
        .child(Icon::Clock.view(12.0, theme::ink3()))
        .child(
            ui::line(format!("Next: {line}"), text::META, theme::ink2())
                .flex_1()
                .min_w_0(),
        )
        .child(
            ui::button("queued-cancel", "Cancel")
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.queued.clear();
                    cx.notify();
                })),
        )
        .with_animation("queued", motion::animation(scene::APPEAR), |row, t| {
            row.opacity(t)
        })
}

/// Stop while Kerf works (its tip "Stopping…" once pressed, `stopping`);
/// else Send, lit while there are words and nothing refuses them, its tip
/// saying why not (`refused`).
fn send_or_stop(
    busy: bool,
    stopping: bool,
    empty: bool,
    refused: Option<String>,
    cx: &Context<ChatView>,
) -> AnyElement {
    if busy {
        icon_button("stop", Icon::Stop)
            .tooltip(if stopping { "Stopping…" } else { "Stop Kerf" })
            .key("Esc")
            .color(theme::ink())
            .disabled(stopping)
            .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
            .into_any_element()
    } else {
        let live = !empty && refused.is_none();
        let tip = match refused {
            Some(reason) => ui::Tip::new(reason),
            None => ui::Tip::new("Send").key("↩"),
        };
        let send = div()
            .id("send")
            .size(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(if live { theme::accent() } else { theme::on() })
            .when(live, |send| {
                send.cursor_pointer()
                    .hover(|send| send.bg(theme::accent_hover()))
            })
            .child(Icon::Send.view(
                16.0,
                if live {
                    theme::accent_text()
                } else {
                    theme::dis()
                },
            ))
            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)));
        ui::tipped("send-tip", tip, send).into_any_element()
    }
}

/// The stamp: the request goes without the playhead's time, which shows
/// struck through; pressed, the time goes with requests again.
fn stamp_chip(timecode: String, cx: &Context<ChatView>) -> AnyElement {
    let chip = theme::type_style(div(), text::TIMECODE)
        .id("composer-stamp")
        .flex()
        .flex_none()
        .items_center()
        .gap(px(theme::space::S4))
        .h(px(20.0))
        .px(px(theme::space::S6))
        .rounded(px(theme::radius::CONTROL))
        .text_color(theme::ink3())
        .cursor_pointer()
        .hover(|chip| chip.bg(theme::hover2()))
        .child(Icon::Clock.view(12.0, theme::ink3()))
        .child(div().line_through().child(timecode))
        .on_click(cx.listener(|this, _, _, cx| {
            this.session.update(cx, |session, cx| {
                session.prefs.send_playhead = true;
                session.save_prefs();
                cx.notify();
            });
            cx.notify();
        }));
    ui::tipped(
        "composer-stamp-tip",
        ui::Tip::new("Time not sent · click to send"),
        chip,
    )
    .into_any_element()
}

/// The context ring: a 14 pt circle filled clockwise from the top as the
/// conversation fills what a request can hold, in `color`.
fn context_ring(fraction: f32, color: Rgba) -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let across = 14.0;
            let stroke = 2.0;
            let center = bounds.center();
            let radius = (across - stroke) / 2.0;
            let ring = |from: f32, to: f32| {
                let steps = ((to - from) * 72.0).ceil().max(2.0) as usize;
                (0..=steps)
                    .map(|step| {
                        let turn = from + (to - from) * step as f32 / steps as f32;
                        let angle = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                        point(
                            center.x + px(radius * angle.cos()),
                            center.y + px(radius * angle.sin()),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let mut track = gpui::PathBuilder::stroke(px(stroke));
            track.add_polygon(&ring(0.0, 1.0), true);
            if let Ok(path) = track.build() {
                window.paint_path(path, theme::track());
            }
            if fraction > 0.0 {
                let mut arc = gpui::PathBuilder::stroke(px(stroke));
                arc.add_polygon(&ring(0.0, fraction.max(0.03)), false);
                if let Ok(path) = arc.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .size(px(24.0))
}

/// The ring's colour for how full the context is.
fn context_color(level: ContextLevel) -> Rgba {
    match level {
        ContextLevel::Room => theme::ink2(),
        ContextLevel::Filling => theme::warn(),
        ContextLevel::Full => theme::neg(),
    }
}
