use crate::agent_rail::AgentRailAction;
use crate::inspector::InspectorAction;
use crate::media_bin::MediaBinAction;
use crate::theme;
use crate::timeline::{
    TimelineAction, TimelineInteractionState, TimelineSelection, TimelineViewport,
};
use crate::view_model::AppViewModel;
use crate::viewer::ViewerAction;
use egui::{Color32, Frame, RichText, Stroke, ViewportCommand};

#[derive(Clone, Debug, PartialEq)]
pub enum AppAction {
    Refresh,
    AddVideoTrack,
    Undo,
    Redo,
    Inspector(InspectorAction),
    MediaBin(MediaBinAction),
    Timeline(TimelineAction),
    AgentPrompt(String),
    Proposal(AgentRailAction),
    Viewer(ViewerAction),
}

/// Transient editor state. Canonical project state remains owned by `cutd`.
#[derive(Debug, Default)]
pub struct UiState {
    pub timeline_selection: TimelineSelection,
    pub timeline_viewport: TimelineViewport,
    pub timeline_interaction: TimelineInteractionState,
    pub right_panel: RightPanelTab,
    pub prompt_draft: String,
    pub media_import_draft: String,
    pub status: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RightPanelTab {
    #[default]
    Inspector,
    Agent,
    Project,
}

pub fn draw(
    ui: &mut egui::Ui,
    model: &mut AppViewModel,
    state: &mut UiState,
    texture: Option<&egui::TextureHandle>,
    caption: Option<&str>,
) -> Vec<AppAction> {
    theme::apply(ui.ctx());
    ui.ctx().send_viewport_cmd(ViewportCommand::Title(format!(
        "{} · Cut",
        model.project_name
    )));
    let mut actions = Vec::new();
    draw_header(ui, model, state, &mut actions);

    // The edit surface is deliberately allocated first: timeline is the product's
    // primary surface, while media, preview, and agent context support it.
    egui::Panel::bottom("timeline-first-timeline")
        .resizable(true)
        .default_size(360.0)
        .min_size(220.0)
        .frame(panel_frame())
        .show(ui, |ui| {
            let previous_selection = state.timeline_selection.clone();
            if let Some(action) = model.timeline.show(
                ui,
                &mut state.timeline_selection,
                &mut state.timeline_viewport,
                &mut state.timeline_interaction,
                model.viewer.playhead,
            ) {
                match action {
                    TimelineAction::Seek(position) => {
                        actions.push(AppAction::Viewer(ViewerAction::Seek(position)))
                    }
                    action => actions.push(AppAction::Timeline(action)),
                }
            }
            if state.timeline_selection != previous_selection
                && matches!(state.timeline_selection, TimelineSelection::Clip(_))
            {
                state.right_panel = RightPanelTab::Inspector;
            }
        });

    egui::Panel::left("timeline-first-media")
        .resizable(true)
        .default_size(198.0)
        .min_size(160.0)
        .frame(panel_frame())
        .show(ui, |ui| {
            section_label(
                ui,
                format!("MEDIA · {}", model.media_bin.items.len()),
                false,
            );
            if let Some(action) = model.media_bin.show(ui, &mut state.media_import_draft) {
                actions.push(AppAction::MediaBin(action));
            }
        });

    egui::Panel::right("timeline-first-agent")
        .resizable(true)
        .default_size(282.0)
        .min_size(240.0)
        .frame(panel_frame())
        .show(ui, |ui| {
            let proposal_ready = model.agent_rail.pending_proposal_id().is_some();
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut state.right_panel,
                    RightPanelTab::Inspector,
                    "Inspector",
                );
                ui.selectable_value(
                    &mut state.right_panel,
                    RightPanelTab::Agent,
                    if proposal_ready {
                        "Agent · 1"
                    } else {
                        "Agent"
                    },
                );
                ui.selectable_value(&mut state.right_panel, RightPanelTab::Project, "Project");
            });
            ui.separator();
            match state.right_panel {
                RightPanelTab::Inspector => {
                    let selected = model
                        .timeline
                        .selected_clip_for_selection(&state.timeline_selection);
                    section_label(
                        ui,
                        selected
                            .map(|(_, clip)| format!("CLIP · {}", clip.source_label))
                            .unwrap_or_else(|| "INSPECTOR".into()),
                        selected.is_some(),
                    );
                    if let Some(action) = model.inspector.show(ui, selected, model.timeline.fps) {
                        actions.push(AppAction::Inspector(action));
                    }
                }
                RightPanelTab::Agent => {
                    section_label(
                        ui,
                        if proposal_ready {
                            "AGENT · PROPOSAL READY"
                        } else {
                            "AGENT"
                        },
                        proposal_ready,
                    );
                    match model.agent_rail.show(ui, &mut state.prompt_draft) {
                        AgentRailAction::None => {}
                        AgentRailAction::Send(prompt) => {
                            actions.push(AppAction::AgentPrompt(prompt))
                        }
                        AgentRailAction::Abort => {
                            actions.push(AppAction::Proposal(AgentRailAction::Abort))
                        }
                        proposal => actions.push(AppAction::Proposal(proposal)),
                    }
                }
                RightPanelTab::Project => {
                    section_label(ui, "PROJECT", false);
                    model.settings.show(ui);
                }
            }
        });

    egui::CentralPanel::default()
        .frame(
            Frame::new()
                .fill(Color32::from_rgb(5, 5, 5))
                .inner_margin(12.0),
        )
        .show(ui, |ui| {
            if let Some(action) = model.viewer.show(ui, texture, caption) {
                actions.push(AppAction::Viewer(action));
            }
        });

    if let Some(status) = state.status.clone() {
        egui::Area::new("status".into())
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
            .show(ui, |ui| {
                Frame::new()
                    .fill(Color32::from_gray(20))
                    .stroke(Stroke::new(1.0, theme::LINE))
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(status).color(Color32::WHITE));
                            if ui.small_button("×").clicked() {
                                state.status = None;
                            }
                        });
                    });
            });
    }
    actions
}

fn draw_header(
    ui: &mut egui::Ui,
    model: &AppViewModel,
    state: &mut UiState,
    actions: &mut Vec<AppAction>,
) {
    egui::Panel::top("timeline-first-header")
        .exact_size(44.0)
        .frame(panel_frame())
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("Cut").size(15.0).strong());
                ui.add_space(10.0);
                ui.label(&model.project_name);
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
                if ui.button("+ Track").clicked() {
                    actions.push(AppAction::AddVideoTrack);
                }
                if ui.button("Refresh").clicked() {
                    actions.push(AppAction::Refresh);
                }
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{} / {}",
                        format_clock(model.viewer.playhead),
                        format_clock(model.duration)
                    ))
                    .monospace(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Project").clicked() {
                        state.right_panel = RightPanelTab::Project;
                    }
                    ui.label(
                        RichText::new(format!(
                            "REV {} · {}",
                            model.revision, model.projection_status
                        ))
                        .small()
                        .color(theme::LIME),
                    );
                });
            });
        });
}

fn panel_frame() -> Frame {
    Frame::new()
        .fill(Color32::BLACK)
        .stroke(Stroke::new(1.0, theme::LINE))
        .inner_margin(egui::Margin::same(10))
}

fn section_label(ui: &mut egui::Ui, text: impl Into<String>, active: bool) {
    ui.label(
        RichText::new(text.into())
            .small()
            .monospace()
            .color(if active { theme::LIME } else { theme::MUTED }),
    );
    ui.add_space(5.0);
}

fn format_clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3_600,
        total / 60 % 60,
        total % 60
    )
}
