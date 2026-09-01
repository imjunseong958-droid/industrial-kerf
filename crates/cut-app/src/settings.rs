use egui::{Color32, RichText};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsViewModel {
    pub project_directory: String,
    pub endpoint: String,
    pub actor: String,
}

impl SettingsViewModel {
    pub fn new(project_directory: &Path, endpoint: String, actor: &str) -> Self {
        Self {
            project_directory: project_directory.display().to_string(),
            endpoint,
            actor: actor.to_owned(),
        }
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Settings").color(Color32::WHITE));
        ui.add_space(8.0);
        ui.label(RichText::new("Project directory").small());
        ui.monospace(&self.project_directory);
        ui.add_space(6.0);
        ui.label(RichText::new("cutd endpoint").small());
        ui.monospace(&self.endpoint);
        ui.add_space(6.0);
        ui.label(RichText::new("Actor").small());
        ui.monospace(&self.actor);
    }
}
