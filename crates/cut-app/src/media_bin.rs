use egui::{Color32, RichText, ScrollArea};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaItemViewModel {
    pub id: String,
    pub label: String,
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaBinViewModel {
    pub items: Vec<MediaItemViewModel>,
}

impl MediaBinViewModel {
    pub fn from_snapshot(snapshot: &cut_runtime::StateSnapshot) -> Self {
        let items = snapshot
            .sources
            .iter()
            .filter_map(|source| {
                let id = source.get("id")?.as_str()?.to_owned();
                let path = source
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown path")
                    .to_owned();
                let label = path
                    .rsplit(['/', '\\'])
                    .next()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&path)
                    .to_owned();
                Some(MediaItemViewModel { id, label, path })
            })
            .collect();
        Self { items }
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Media bin").color(Color32::WHITE));
        ui.add_space(6.0);
        if self.items.is_empty() {
            ui.label(RichText::new("No media imported").color(Color32::from_gray(145)));
            return;
        }
        ScrollArea::vertical().show(ui, |ui| {
            for item in &self.items {
                ui.label(RichText::new(&item.label).strong());
                ui.label(
                    RichText::new(&item.path)
                        .small()
                        .color(Color32::from_gray(130)),
                );
                ui.separator();
            }
        });
    }
}
