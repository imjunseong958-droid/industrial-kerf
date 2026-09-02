use crate::timeline::{MediaType, SourceViewModel, TimelineViewModel};
use egui::{Color32, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// A media-bin action that crosses the desktop/runtime boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaBinAction {
    Import(String),
}

/// A source projection used by the media bin. It contains metadata and cache
/// status only; media bytes remain outside the desktop view model.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaItemViewModel {
    pub id: String,
    pub label: String,
    pub path: String,
    pub media_type: MediaType,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_audio: Option<bool>,
    pub thumbnail_path: Option<String>,
    pub thumbnail_available: bool,
    pub waveform_available: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaBinViewModel {
    pub items: Vec<MediaItemViewModel>,
}

impl MediaBinViewModel {
    /// Build the bin from the same typed source projection as the timeline.
    pub fn from_timeline(timeline: &TimelineViewModel, project_directory: Option<&Path>) -> Self {
        let items = timeline
            .sources
            .iter()
            .map(|source| media_item(source, project_directory))
            .collect();
        Self { items }
    }

    /// Compatibility constructor for callers that do not need cache lookup.
    pub fn from_snapshot(snapshot: &cut_runtime::StateSnapshot) -> Self {
        let timeline = TimelineViewModel::from_snapshot(snapshot);
        Self::from_timeline(&timeline, None)
    }

    /// Draw the bin and return an import action when the user submits a path.
    pub fn show(&self, ui: &mut egui::Ui, import_path: &mut String) -> Option<MediaBinAction> {
        ui.heading(RichText::new("Media bin").color(Color32::WHITE));
        ui.add_space(5.0);
        let dropped_path = ui.input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path())
                .map(|path| path.to_string_lossy().into_owned())
        });
        let drop_zone = ui.add_sized(
            [ui.available_width(), 34.0],
            egui::Button::new("Drop a media file here").frame(false),
        );
        if !ui.input(|input| input.raw.hovered_files.is_empty()) {
            ui.painter().rect_stroke(
                drop_zone.rect,
                0.0,
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
        if let Some(path) = dropped_path {
            return Some(MediaBinAction::Import(path));
        }
        let mut submit = false;
        ui.horizontal(|ui| {
            let response = ui.add(
                TextEdit::singleline(import_path)
                    .hint_text("Or paste a media path")
                    .desired_width((ui.available_width() - 66.0).max(40.0)),
            );
            submit = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui
                .add_enabled(!import_path.trim().is_empty(), egui::Button::new("Import"))
                .clicked()
            {
                submit = true;
            }
        });
        if submit && !import_path.trim().is_empty() {
            return Some(MediaBinAction::Import(import_path.trim().to_owned()));
        }

        ui.add_space(6.0);
        if self.items.is_empty() {
            ui.label(RichText::new("No media imported").color(Color32::from_gray(145)));
            return None;
        }

        ScrollArea::vertical().show(ui, |ui| {
            for item in &self.items {
                ui.separator();
                ui.horizontal_top(|ui| {
                    paint_thumbnail(ui, item);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&item.label).strong());
                        ui.label(
                            RichText::new(metadata_line(item))
                                .small()
                                .color(Color32::from_gray(190)),
                        );
                        ui.label(
                            RichText::new(availability_line(item))
                                .small()
                                .color(Color32::from_gray(135)),
                        );
                        ui.label(
                            RichText::new(&item.path)
                                .small()
                                .color(Color32::from_gray(105)),
                        );
                    });
                });
            }
        });
        None
    }
}

fn media_item(source: &SourceViewModel, project_directory: Option<&Path>) -> MediaItemViewModel {
    let explicit_thumbnail = source
        .thumbnail_path
        .as_deref()
        .map(|path| resolve_optional_path(project_directory, path))
        .filter(|path| path.is_file());
    let cached_thumbnail = project_directory.and_then(|directory| {
        find_cached_artifact(
            directory,
            &source.path,
            &["filmstrip", "frames"],
            &["filmstrip.png", "frame-000.png"],
        )
    });
    let thumbnail_path = explicit_thumbnail
        .clone()
        .or(cached_thumbnail.clone())
        .map(|path| path.to_string_lossy().into_owned());

    let explicit_waveform = source
        .waveform_path
        .as_deref()
        .map(|path| resolve_optional_path(project_directory, path))
        .filter(|path| path.is_file());
    let cached_waveform = project_directory.and_then(|directory| {
        find_cached_artifact(
            directory,
            &source.path,
            &["waveform"],
            &["waveform.png", "waveform.json"],
        )
    });
    let waveform_available = source
        .waveform_available
        .or_else(|| explicit_waveform.as_ref().map(|_| true))
        .or_else(|| cached_waveform.as_ref().map(|_| true))
        .or_else(|| {
            source
                .has_audio
                .map(|has_audio| has_audio && cached_waveform.is_some())
        });

    MediaItemViewModel {
        id: source.id.clone(),
        label: source.label.clone(),
        path: source.path.clone(),
        media_type: source.media_type,
        duration: source.duration,
        width: source.width,
        height: source.height,
        has_audio: source.has_audio,
        thumbnail_path,
        thumbnail_available: explicit_thumbnail.is_some() || cached_thumbnail.is_some(),
        waveform_available,
    }
}

fn resolve_optional_path(project_directory: Option<&Path>, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_directory
            .map(|directory| directory.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    }
}

fn find_cached_artifact(
    project_directory: &Path,
    source_path: &str,
    kinds: &[&str],
    artifacts: &[&str],
) -> Option<PathBuf> {
    let source_path = resolve_project_path(project_directory, source_path);
    let canonical_source = source_path.canonicalize().ok()?;
    let source_metadata = fs::metadata(&canonical_source).ok()?;
    let source_identity = canonical_source.to_string_lossy().into_owned();
    let source_size = source_metadata.len();
    let source_modified_ns = source_metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok());
    let root = project_directory.join(".cut").join("cache").join("derived");
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !kinds.iter().any(|kind| entry_name_has_kind(&path, kind)) {
            continue;
        }
        let manifest = path.join("manifest.json");
        let Ok(bytes) = fs::read(manifest) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let identity = value.get("identity").and_then(serde_json::Value::as_object);
        let identity_matches = identity
            .and_then(|identity| identity.get("path"))
            .and_then(serde_json::Value::as_str)
            == Some(source_identity.as_str())
            && identity
                .and_then(|identity| identity.get("size"))
                .and_then(serde_json::Value::as_u64)
                == Some(source_size)
            && identity
                .and_then(|identity| identity.get("modifiedNs"))
                .or_else(|| identity.and_then(|identity| identity.get("modified_ns")))
                .and_then(serde_json::Value::as_u64)
                == source_modified_ns;
        if !identity_matches {
            continue;
        }
        if let Some(artifact) = artifacts
            .iter()
            .map(|artifact| path.join(artifact))
            .find(|artifact| artifact.is_file())
        {
            return Some(artifact);
        }
    }
    None
}

fn entry_name_has_kind(path: &Path, kind: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == kind || name.starts_with(&format!("{kind}-")))
}

fn resolve_project_path(project_directory: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_directory.join(path)
    }
}

fn paint_thumbnail(ui: &mut egui::Ui, item: &MediaItemViewModel) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 48.0), Sense::hover());
    let color = match item.media_type {
        MediaType::Video => Color32::from_rgb(57, 126, 206),
        MediaType::Audio => Color32::from_rgb(190, 125, 48),
        MediaType::Image => Color32::from_rgb(70, 161, 125),
        MediaType::Text => Color32::from_rgb(147, 93, 184),
        MediaType::Unknown => Color32::from_gray(105),
    };
    ui.painter().rect_filled(rect, 0.0, Color32::from_gray(18));
    ui.painter()
        .rect_stroke(rect, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    let label = if item.thumbnail_available {
        "THUMB".to_owned()
    } else {
        item.media_type.as_str().to_uppercase()
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(9.0),
        color,
    );
}

fn metadata_line(item: &MediaItemViewModel) -> String {
    let duration = item
        .duration
        .map(format_duration)
        .unwrap_or_else(|| "duration ?".into());
    let dimensions = match (item.width, item.height) {
        (Some(width), Some(height)) => format!("{width}×{height}"),
        _ => "dimensions ?".into(),
    };
    let audio = match item.has_audio {
        Some(true) => "audio",
        Some(false) => "no audio",
        None => "audio ?",
    };
    format!("{duration} · {dimensions} · {audio}")
}

fn availability_line(item: &MediaItemViewModel) -> String {
    let thumbnail = if item.thumbnail_available {
        "thumbnail ready"
    } else {
        "thumbnail not generated"
    };
    let waveform = match item.waveform_available {
        Some(true) => "waveform ready",
        Some(false) => "waveform unavailable",
        None => "waveform not generated",
    };
    format!("{thumbnail} · {waveform}")
}

fn format_duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "duration ?".into();
    }
    let total = seconds.round() as u64;
    let hours = total / 3_600;
    let minutes = (total % 3_600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::{availability_line, format_duration, MediaBinViewModel};
    use crate::timeline::TimelineViewModel;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn media_projection_keeps_probe_metadata_and_cache_status() {
        let timeline = TimelineViewModel::from_project(&json!({
            "version": 2,
            "fps": 30,
            "sources": [{
                "id": "source-1",
                "path": "shot.mp4",
                "duration": 12.5,
                "width": 1920,
                "height": 1080,
                "hasAudio": true,
                "waveformAvailable": true
            }],
            "tracks": []
        }));
        let model = MediaBinViewModel::from_timeline(&timeline, Some(Path::new("/tmp")));
        let item = &model.items[0];
        assert_eq!(item.width, Some(1920));
        assert_eq!(item.height, Some(1080));
        assert_eq!(item.has_audio, Some(true));
        assert_eq!(item.waveform_available, Some(true));
        assert!(availability_line(item).contains("waveform ready"));
    }

    #[test]
    fn duration_display_is_bounded_and_stable() {
        assert_eq!(format_duration(0.0), "00:00");
        assert_eq!(format_duration(65.4), "01:05");
        assert_eq!(format_duration(3_665.0), "1:01:05");
    }
}
