use crate::timeline::TrackViewModel;
use egui::{Color32, RichText, Slider};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InspectorViewModel;

#[derive(Clone, Debug, PartialEq)]
pub enum InspectorAction {
    SetClipVolume { clip_id: String, volume: f64 },
}

impl InspectorViewModel {
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        selected: Option<&TrackViewModel>,
    ) -> Option<InspectorAction> {
        ui.heading(RichText::new("Inspector").color(Color32::WHITE));
        ui.add_space(8.0);
        let mut action = None;
        match selected {
            Some(track) => {
                ui.label(RichText::new(&track.name).strong());
                ui.label(format!("ID  {}", track.id));
                ui.label(format!("Type  {}", track.kind));
                ui.label(format!("Clips  {}", track.clip_count));
                ui.label(if track.muted {
                    "Audio  muted"
                } else {
                    "Audio  active"
                });
                ui.label(if track.enabled {
                    "Track  enabled"
                } else {
                    "Track  disabled"
                });
                if let Some(clip_id) = &track.first_clip_id {
                    ui.separator();
                    ui.label(RichText::new(format!("Clip  {clip_id}")).small());
                    let mut volume = track.first_clip_volume;
                    if ui
                        .add(Slider::new(&mut volume, 0.0..=2.0).text("Volume"))
                        .changed()
                    {
                        action = Some(InspectorAction::SetClipVolume {
                            clip_id: clip_id.clone(),
                            volume,
                        });
                    }
                }
            }
            None => {
                ui.label("Select a track to inspect");
            }
        }
        action
    }
}
