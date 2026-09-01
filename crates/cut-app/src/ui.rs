use crate::agent_rail::AgentRailAction;
use crate::inspector::InspectorAction;
use crate::view_model::AppViewModel;
use crate::viewer::ViewerAction;
use egui::{CentralPanel, Color32, RichText, ViewportCommand};

#[derive(Clone, Debug, PartialEq)]
pub enum AppAction {
    Refresh,
    AddVideoTrack,
    Undo,
    Redo,
    Inspector(InspectorAction),
    AgentPrompt(String),
    Proposal(AgentRailAction),
    Viewer(ViewerAction),
}

/// State owned by the shell rather than by `cutd`: focus-adjacent selection,
/// panel visibility, the prompt draft, and transient status text.
#[derive(Debug, Default)]
pub struct UiState {
    pub selected_track: Option<String>,
    pub show_settings: bool,
    pub prompt_draft: String,
    pub status: Option<String>,
}

pub fn draw(
    ui: &mut egui::Ui,
    model: &mut AppViewModel,
    state: &mut UiState,
    texture: Option<&egui::TextureHandle>,
    caption: Option<&str>,
) -> Vec<AppAction> {
    ui.ctx().send_viewport_cmd(ViewportCommand::Title(format!(
        "{} · Cut",
        model.project_name
    )));
    let mut actions = Vec::new();

    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new(&model.project_name).color(Color32::WHITE));
            ui.separator();
            ui.label(format!("revision {}", model.revision));
            ui.label(format!("{:.1}s", model.duration));
            ui.separator();
            if ui
                .add_enabled(model.can_undo, egui::Button::new("Undo"))
                .clicked()
            {
                actions.push(AppAction::Undo);
            }
            if ui
                .add_enabled(model.can_redo, egui::Button::new("Redo"))
                .clicked()
            {
                actions.push(AppAction::Redo);
            }
            if ui.button("Add video track").clicked() {
                actions.push(AppAction::AddVideoTrack);
            }
            if ui.button("Refresh").clicked() {
                actions.push(AppAction::Refresh);
            }
            if ui
                .button(if state.show_settings {
                    "Editor"
                } else {
                    "Settings"
                })
                .clicked()
            {
                state.show_settings = !state.show_settings;
            }
            ui.separator();
            ui.label(
                RichText::new(format!("projection: {}", model.projection_status))
                    .small()
                    .color(Color32::from_gray(150)),
            );
        });
    });

    egui::Panel::left("media-bin")
        .resizable(true)
        .default_size(220.0)
        .show(ui, |ui| model.media_bin.show(ui));

    egui::Panel::right("inspector-and-agent")
        .resizable(true)
        .default_size(300.0)
        .show(ui, |ui| {
            if state.show_settings {
                model.settings.show(ui);
            } else {
                let selected = model.timeline.selected(state.selected_track.as_deref());
                if let Some(action) = model.inspector.show(ui, selected) {
                    actions.push(AppAction::Inspector(action));
                }
                ui.separator();
                let action = model.agent_rail.show(ui, &mut state.prompt_draft);
                match action {
                    AgentRailAction::None => {}
                    AgentRailAction::Send(prompt) => actions.push(AppAction::AgentPrompt(prompt)),
                    proposal => actions.push(AppAction::Proposal(proposal)),
                }
            }
        });

    egui::Panel::bottom("timeline")
        .resizable(true)
        .default_size(220.0)
        .show(ui, |ui| model.timeline.show(ui, &mut state.selected_track));

    CentralPanel::default().show(ui, |ui| {
        if let Some(action) = model.viewer.show(ui, texture, caption) {
            actions.push(AppAction::Viewer(action));
        }
    });

    if let Some(status) = &state.status {
        egui::Area::new("status".into())
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(status)
                        .background_color(Color32::from_gray(28))
                        .color(Color32::WHITE),
                );
            });
    }
    actions
}
