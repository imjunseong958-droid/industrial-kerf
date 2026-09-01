use egui::{Color32, RichText, ScrollArea};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct TrackViewModel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub clip_count: usize,
    pub first_clip_id: Option<String>,
    pub first_clip_volume: f64,
    pub enabled: bool,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineViewModel {
    pub tracks: Vec<TrackViewModel>,
}

impl TimelineViewModel {
    pub fn from_project(project: &Value) -> Self {
        let tracks = project
            .get("tracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(track_from_value)
            .collect();
        Self { tracks }
    }

    pub fn show(&self, ui: &mut egui::Ui, selected_track: &mut Option<String>) {
        ui.heading(RichText::new("Timeline").color(Color32::WHITE));
        ui.add_space(6.0);
        if self.tracks.is_empty() {
            ui.label("No tracks");
            return;
        }
        ScrollArea::vertical().show(ui, |ui| {
            for track in &self.tracks {
                let selected = selected_track.as_deref() == Some(track.id.as_str());
                let response = ui.selectable_label(
                    selected,
                    RichText::new(format!("{}  ·  {}", track.name, track.kind.to_uppercase()))
                        .color(if selected {
                            Color32::WHITE
                        } else {
                            Color32::from_gray(190)
                        }),
                );
                if response.clicked() {
                    *selected_track = Some(track.id.clone());
                }
                ui.label(
                    RichText::new(format!(
                        "{} clip{}{}",
                        track.clip_count,
                        if track.clip_count == 1 { "" } else { "s" },
                        if track.muted { " · muted" } else { "" }
                    ))
                    .small()
                    .color(Color32::from_gray(115)),
                );
                ui.separator();
            }
        });
    }

    pub fn selected(&self, selected_track: Option<&str>) -> Option<&TrackViewModel> {
        selected_track.and_then(|id| self.tracks.iter().find(|track| track.id == id))
    }
}

fn track_from_value(value: &Value) -> Option<TrackViewModel> {
    let id = value.get("id")?.as_str()?.to_owned();
    let clips = value.get("clips").and_then(Value::as_array);
    let first_clip = clips.and_then(|clips| clips.first());
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("track")
        .to_owned();
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&kind)
        .to_owned();
    Some(TrackViewModel {
        id,
        name,
        kind,
        clip_count: clips.map_or(0, Vec::len),
        first_clip_id: first_clip
            .and_then(|clip| clip.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        first_clip_volume: first_clip
            .and_then(|clip| clip.get("volume"))
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 2.0),
        enabled: value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        muted: value.get("muted").and_then(Value::as_bool).unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::TimelineViewModel;
    use serde_json::json;

    #[test]
    fn derives_track_rows_without_retaining_the_project_json() {
        let model = TimelineViewModel::from_project(&json!({
            "tracks": [{"id":"video-1","type":"video","clips":[{}]}]
        }));
        assert_eq!(model.tracks[0].id, "video-1");
        assert_eq!(model.tracks[0].clip_count, 1);
    }
}
