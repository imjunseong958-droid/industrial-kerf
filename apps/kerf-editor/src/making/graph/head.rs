//! The graph's head: back from a model's workflow, the shot's name and
//! where its steps are, how it stands, and Stop, or Add step, Several and
//! Make (Make again) with the menus they open; and Close.

use super::{MenuKind, ShotGraph};
use crate::icons::Icon;
use crate::making::{Recipe, HEAD};
use crate::store::Making;
use crate::theme::{self, text};
use crate::ui::{self, icon_button};
use gpui::{
    div, point, prelude::*, px, AnyElement, Context, MouseDownEvent, Pixels, Point, Rgba, Window,
};
use kerf_bridge::{Command, RecipeEdit};

impl ShotGraph {
    /// How the shot stands, the most pressing thing first, and what its tip says.
    fn status(&self, making: Option<&Making>) -> (String, Rgba, String) {
        if let Some(making) = making {
            let said = if making.line.is_empty() {
                format!("Making take {}…", making.take)
            } else {
                format!("Making take {}: {}", making.take, making.line)
            };
            return (said.clone(), theme::activity::gpu(), said);
        }
        let Some(recipe) = &self.recipe else {
            return match &self.refused {
                Some(why) => ("Cannot be shown".into(), theme::neg(), why.clone()),
                None => ("Reading its steps…".into(), theme::ink3(), String::new()),
            };
        };
        if let Some(error) = &recipe.error {
            let said = if recipe.document {
                "The steps do not read"
            } else {
                "The recipe does not read"
            };
            return (said.into(), theme::neg(), error.clone());
        }
        if let Some(cannot) = &recipe.cannot {
            return (
                "Cannot be made as things stand".into(),
                theme::neg(),
                cannot.clone(),
            );
        }
        let takes = recipe.node("takes");
        if let Some(wants) = takes.and_then(|takes| takes.holds["wants"].as_str()) {
            let said = format!("Wants making: {wants}");
            return (said.clone(), theme::warn(), said);
        }
        let said = takes.map(|takes| takes.detail.clone()).unwrap_or_default();
        (said.clone(), theme::ink3(), said)
    }

    /// The head: back from a model's workflow, the shot's name and where
    /// its steps are, how it stands (its tip says all of it), what the
    /// making can do, and Close.
    pub(super) fn head(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let making = self.making(session).cloned();
        let shot_name = session.media_name(&self.media);
        let (status, status_color, status_tip) = self.status(making.as_ref());
        let name = self
            .recipe
            .as_ref()
            .map(|recipe| recipe.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or(shot_name);
        let path = match (&self.workflow, &self.recipe) {
            (Some(workflow), _) => format!(
                "{} · {}",
                workflow.model,
                if workflow.is_own() {
                    &workflow.file
                } else {
                    &workflow.name
                }
            ),
            (None, Some(recipe)) if recipe.document => "steps in the document".to_owned(),
            (None, Some(recipe)) => recipe.recipe.clone(),
            (None, None) => String::new(),
        };
        let mut head = div()
            .h(px(HEAD))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme::space::S8))
            .pl(px(if self.workflow.is_some() { 10.0 } else { 16.0 }))
            .pr(px(theme::space::S12))
            .border_b_1()
            .border_color(theme::line());
        if self.workflow.is_some() {
            head = head.child(
                icon_button("graph-back", Icon::ChevronLeft)
                    .tooltip("Back to the shot's graph")
                    .on_click(cx.listener(|this, _, window, cx| this.close_workflow(window, cx))),
            );
        }
        head = head
            .child(Icon::Media.view(16.0, theme::ink2()))
            .child(
                ui::line(name, text::TITLE, theme::ink())
                    .flex_none()
                    .max_w(px(240.0)),
            )
            .when(!path.is_empty(), |head| {
                head.child(
                    ui::line(path, text::MONO_SMALL, theme::ink3())
                        .flex_shrink()
                        .min_w_0()
                        .max_w(px(320.0)),
                )
            })
            .child(ui::tip_on(
                div()
                    .id("graph-status")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_end()
                    .child(ui::line(status, text::CONTROL, status_color)),
                (!status_tip.is_empty()).then(|| ui::Tip::new(status_tip)),
            ));
        head.children(self.making_buttons(making.is_some(), cx))
            .child(
                icon_button("graph-close", Icon::Close)
                    .tooltip("Close")
                    .key("Esc")
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
            )
            .into_any_element()
    }

    /// While the shot is being made, Stop; else Add step (on the shot's own
    /// graph, not a model's workflow), Several and Make, each greyed with
    /// why while it cannot run.
    fn making_buttons(&self, making: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if making {
            return vec![ui::button("graph-stop", "Stop")
                .icon(Icon::Stop)
                .tooltip("Stop making: its step is asked to end, and no take is kept")
                .on_click(cx.listener(move |this, _, _, cx| {
                    let media = this.media.to_string();
                    this.session.update(cx, |session, _| {
                        session.send(Command::MediaMakeCancel { media });
                    });
                }))
                .into_any_element()];
        }
        let connected =
            self.session.read(cx).state.connection == crate::store::Connection::Connected;
        let reads = self.recipe.as_ref().is_some_and(Recipe::reads);
        let document = self.recipe.as_ref().is_none_or(|recipe| recipe.document);
        let has_takes = self
            .recipe
            .as_ref()
            .and_then(|recipe| recipe.node("takes"))
            .and_then(|takes| takes.holds["takes"].as_array())
            .is_some_and(|takes| !takes.is_empty());
        let can = connected && reads;
        let open_menu = self.menu.as_ref().map(|open| open.kind);
        let why = if !connected {
            "Kerf is not connected".to_owned()
        } else if self.recipe.is_none() {
            "Its steps are still being read".to_owned()
        } else if document {
            "The steps do not read".to_owned()
        } else {
            "The recipe does not read".to_owned()
        };
        let mut buttons = Vec::new();
        if self.workflow.is_none() {
            // A step is added to the shot's steps, not to a model's workflow, and to ones that read.
            let can_add = reads;
            buttons.push(
                ui::trigger(
                    "graph-add-step-trigger",
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        if can_add {
                            this.commit_fields(cx);
                            let at = event.position + point(px(0.0), px(16.0));
                            this.toggle_menu(MenuKind::AddStep, at, window, cx);
                        }
                    }),
                )
                .child(
                    ui::button("graph-add-step", "Add step")
                        .ghost()
                        .icon(Icon::Plus)
                        .disabled(!can_add)
                        .when(open_menu != Some(MenuKind::AddStep), |button| {
                            button
                                .tooltip("A new step, written to read as it stands, then set in its card")
                                .reason(why.clone())
                        }),
                )
                .into_any_element(),
            );
        }
        buttons.push(
            ui::trigger(
                "graph-several-trigger",
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    if can {
                        this.commit_fields(cx);
                        let at = event.position + point(px(0.0), px(16.0));
                        this.toggle_menu(MenuKind::Several, at, window, cx);
                    }
                }),
            )
            .child(
                ui::button("graph-several", "Several")
                    .ghost()
                    .icon(Icon::ChevronDown)
                    .disabled(!can)
                    .when(open_menu != Some(MenuKind::Several), |button| {
                        button
                            .tooltip("Make two to four takes to choose among")
                            .reason(why.clone())
                    }),
            )
            .into_any_element(),
        );
        buttons.push(
            ui::button("graph-make", if has_takes { "Make again" } else { "Make" })
                .primary()
                .disabled(!can)
                .tooltip(if document {
                    "Run its steps and write a new take; what it runs is shown first"
                } else {
                    "Run the recipe and write a new take; what it runs is shown first"
                })
                .reason(why)
                .on_click(cx.listener(|this, _, window, cx| this.open_plan(1, window, cx)))
                .into_any_element(),
        );
        buttons
    }

    /// The head's menus open from a press, and a second press puts them away.
    fn toggle_menu(
        &mut self,
        kind: MenuKind,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_menu(kind, window, cx) {
            return;
        }
        let this = cx.weak_entity();
        let (items, header): (Vec<ui::MenuItem>, &str) = match kind {
            MenuKind::Several => (
                // Several takes of the same shot to choose among, each with its seeds one on.
                [2u32, 3, 4]
                    .into_iter()
                    .map(|rolls| {
                        let this = this.clone();
                        ui::MenuItem::new(format!("{rolls} Takes…")).on_choose(move |window, cx| {
                            let _ = this.update(cx, |graph, cx| graph.open_plan(rolls, window, cx));
                        })
                    })
                    .collect(),
                "Each seed one on from the last",
            ),
            MenuKind::AddStep => (
                [
                    ("guide", "Guide, drawn from a sequence"),
                    ("model", "Model, asked for a file"),
                    ("join", "Join, parts set end to end"),
                ]
                .into_iter()
                .map(|(kind, words)| {
                    let this = this.clone();
                    ui::MenuItem::new(words).on_choose(move |_, cx| {
                        let _ = this.update(cx, |graph, cx| {
                            graph.edit(
                                RecipeEdit {
                                    add: Some(kind.to_owned()),
                                    ..RecipeEdit::default()
                                },
                                cx,
                            )
                        });
                    })
                })
                .collect(),
                if self.recipe.as_ref().is_some_and(|recipe| !recipe.document) {
                    "A new line of the recipe"
                } else {
                    "A new step of the shot"
                },
            ),
            _ => return,
        };
        self.open_menu(kind, items, Some(header.to_owned()), at, window, cx);
    }
}
