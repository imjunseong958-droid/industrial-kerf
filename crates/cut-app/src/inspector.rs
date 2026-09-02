use crate::timeline::{ClipViewModel, TrackViewModel};
use cut_core::capability::{capability, CapabilitySupport, EditCapability};
use cut_core::{FrameGrid, FrameIndex};
use cut_domain::{
    ColorEffect, ColorEffectParams, CropRegion, Effect, JsonValue, Keyframe, Transform, Transition,
};
use egui::{Color32, ComboBox, DragValue, RichText};
use serde_json::{json, Number};

/// The inspector is intentionally stateless. Draft project data stays in the
/// authoritative snapshot; this type only renders a selected clip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InspectorViewModel;

/// Typed edits emitted by the inspector. The app converts each variant into
/// one canonical command sent to `cutd`.
#[derive(Clone, Debug, PartialEq)]
pub enum InspectorAction {
    SetTimelineStart {
        clip_id: String,
        timeline_frame: FrameIndex,
    },
    SetTimelineDuration {
        clip_id: String,
        timeline_start_frame: FrameIndex,
        duration_frame: FrameIndex,
    },
    SetSourceRange {
        clip_id: String,
        source_in_frame: FrameIndex,
        source_out_frame: FrameIndex,
    },
    SetSpeed {
        clip_id: String,
        speed: f64,
    },
    SetVolume {
        clip_id: String,
        volume: f64,
    },
    SetCrop {
        clip_id: String,
        crop: CropRegion,
    },
    ClearCrop {
        clip_id: String,
    },
    SetTransform {
        clip_id: String,
        transform: Box<Transform>,
    },
    ClearTransform {
        clip_id: String,
    },
    SetOpacity {
        clip_id: String,
        opacity: f64,
    },
    SetEffects {
        clip_id: String,
        effects: Vec<Effect>,
    },
    SetKeyframes {
        clip_id: String,
        keyframes: Vec<Keyframe>,
    },
    SetTransitions {
        clip_id: String,
        transitions: Vec<Transition>,
    },
}

impl InspectorViewModel {
    /// Draw all supported clip controls in the product order from Phase 8.
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        selected: Option<(&TrackViewModel, &ClipViewModel)>,
        fps: f64,
    ) -> Option<InspectorAction> {
        ui.heading(RichText::new("Inspector").color(Color32::WHITE));
        ui.add_space(6.0);
        let Some((track, clip)) = selected else {
            ui.label(RichText::new("Select a clip to inspect").color(Color32::from_gray(145)));
            return None;
        };

        ui.label(RichText::new(&clip.source_label).strong());
        ui.label(
            RichText::new(format!(
                "{} · {} · {}",
                clip.id,
                track.name,
                clip.media_type.as_str()
            ))
            .small()
            .color(Color32::from_gray(145)),
        );
        if track.locked {
            ui.label(
                RichText::new("Track is locked; inspector edits are disabled")
                    .small()
                    .color(Color32::from_rgb(255, 174, 92)),
            );
        }

        let grid = FrameGrid::new(fps).expect("timeline projection always has a positive fps");
        let editable = !track.locked;
        let mut action;

        ui.separator();
        ui.label(RichText::new("Timing and source range").strong());
        capability_note(ui, EditCapability::Placement);
        action = show_timing(ui, clip, grid, editable);
        if action.is_none() {
            action = show_source_range(ui, clip, grid, editable);
        }

        ui.separator();
        ui.label(RichText::new("Playback").strong());
        capability_note(ui, EditCapability::Speed);
        if action.is_none() {
            action = show_speed(ui, clip, editable);
        }
        capability_note(ui, EditCapability::Volume);
        if action.is_none() {
            action = show_volume(ui, clip, editable);
        }

        ui.separator();
        ui.label(RichText::new("Crop").strong());
        capability_note(ui, EditCapability::Crop);
        if track.kind == "title" {
            ui.label(
                RichText::new("Crop is not supported for generated title clips")
                    .small()
                    .color(Color32::from_gray(135)),
            );
        } else if !visual_track(track) {
            ui.label(
                RichText::new("Visual controls are available only for video and title tracks")
                    .small()
                    .color(Color32::from_gray(135)),
            );
        }
        if action.is_none() {
            action = show_crop(ui, clip, editable && track.kind == "video");
        }

        ui.separator();
        ui.label(RichText::new("Transform and opacity").strong());
        capability_note(ui, EditCapability::Transform);
        if track.kind == "title" {
            ui.label(
                RichText::new("Transform is not supported for generated title clips")
                    .small()
                    .color(Color32::from_gray(135)),
            );
        }
        if action.is_none() {
            action = show_transform(ui, clip, editable && track.kind == "video");
        }
        capability_note(ui, EditCapability::Opacity);
        if action.is_none() {
            action = show_opacity(ui, clip, editable && visual_track(track));
        }

        ui.separator();
        ui.label(RichText::new("Color effects").strong());
        capability_note(ui, EditCapability::Effects);
        if action.is_none() {
            action = show_effects(ui, track, clip, editable);
        }

        ui.separator();
        ui.label(RichText::new("Keyframes").strong());
        capability_note(ui, EditCapability::Keyframes);
        if action.is_none() {
            action = show_keyframes(
                ui,
                clip,
                grid,
                editable && visual_track(track),
                track.kind == "title",
            );
        }

        ui.separator();
        ui.label(RichText::new("Transition").strong());
        capability_note(ui, EditCapability::Transitions);
        if action.is_none() {
            action = show_transition(ui, track, clip, editable && visual_track(track));
        }
        action
    }
}

fn show_timing(
    ui: &mut egui::Ui,
    clip: &ClipViewModel,
    grid: FrameGrid,
    editable: bool,
) -> Option<InspectorAction> {
    let mut start = grid.frame_at(clip.timeline.start);
    let start_changed = ui
        .add_enabled(
            editable && supports(EditCapability::Placement),
            DragValue::new(&mut start)
                .speed(1)
                .range(0..=1_000_000_000_i64)
                .prefix("Start ")
                .suffix(" fr"),
        )
        .changed();
    ui.label(
        RichText::new(format!(
            "{:.3}s → {:.3}s",
            clip.timeline.start, clip.timeline.end
        ))
        .small()
        .color(Color32::from_gray(135)),
    );
    if start_changed && start != grid.frame_at(clip.timeline.start) {
        return Some(InspectorAction::SetTimelineStart {
            clip_id: clip.id.clone(),
            timeline_frame: start,
        });
    }

    let mut duration = grid.frame_at(clip.timeline.duration()).max(1);
    let duration_changed = ui
        .add_enabled(
            editable,
            DragValue::new(&mut duration)
                .speed(1)
                .range(1..=1_000_000_000_i64)
                .prefix("Length ")
                .suffix(" fr"),
        )
        .changed();
    if duration_changed && duration != grid.frame_at(clip.timeline.duration()).max(1) {
        return Some(InspectorAction::SetTimelineDuration {
            clip_id: clip.id.clone(),
            timeline_start_frame: grid.frame_at(clip.timeline.start),
            duration_frame: duration,
        });
    }
    None
}

fn show_source_range(
    ui: &mut egui::Ui,
    clip: &ClipViewModel,
    grid: FrameGrid,
    editable: bool,
) -> Option<InspectorAction> {
    let Some(source) = clip.source else {
        ui.label(
            RichText::new("Source range unavailable for generated media")
                .small()
                .color(Color32::from_gray(135)),
        );
        return None;
    };
    let mut source_in = grid.frame_at(source.start);
    let mut source_out = grid.frame_at(source.end).max(source_in + 1);
    let original_in = source_in;
    let original_out = source_out;
    ui.horizontal(|ui| {
        ui.label("Source");
        ui.label(format!("{:.3}s – {:.3}s", source.start, source.end));
    });
    let maximum = clip.source_duration.map(|duration| grid.frame_at(duration));
    let in_changed = if let Some(maximum) = maximum {
        ui.add_enabled(
            editable && supports(EditCapability::SourceRange),
            DragValue::new(&mut source_in)
                .speed(1)
                .range(0..=maximum.saturating_sub(1))
                .prefix("In ")
                .suffix(" fr"),
        )
        .changed()
    } else {
        ui.add_enabled(
            editable && supports(EditCapability::SourceRange),
            DragValue::new(&mut source_in)
                .speed(1)
                .range(0..=1_000_000_000_i64)
                .prefix("In ")
                .suffix(" fr"),
        )
        .changed()
    };
    let out_changed = if let Some(maximum) = maximum {
        ui.add_enabled(
            editable && supports(EditCapability::SourceRange),
            DragValue::new(&mut source_out)
                .speed(1)
                .range(1..=maximum.max(1))
                .prefix("Out ")
                .suffix(" fr"),
        )
        .changed()
    } else {
        ui.add_enabled(
            editable && supports(EditCapability::SourceRange),
            DragValue::new(&mut source_out)
                .speed(1)
                .range(1..=1_000_000_000_i64)
                .prefix("Out ")
                .suffix(" fr"),
        )
        .changed()
    };
    if source_in >= source_out {
        ui.label(
            RichText::new("In must be before Out")
                .small()
                .color(Color32::from_rgb(255, 130, 130)),
        );
        return None;
    }
    if (in_changed || out_changed) && (source_in != original_in || source_out != original_out) {
        return Some(InspectorAction::SetSourceRange {
            clip_id: clip.id.clone(),
            source_in_frame: source_in,
            source_out_frame: source_out,
        });
    }
    None
}

fn show_speed(ui: &mut egui::Ui, clip: &ClipViewModel, editable: bool) -> Option<InspectorAction> {
    let mut speed = clip.speed;
    if ui
        .add_enabled(
            editable && supports(EditCapability::Speed),
            DragValue::new(&mut speed)
                .speed(0.01)
                .range(0.01..=100.0)
                .prefix("Speed ")
                .suffix("×"),
        )
        .changed()
        && speed.is_finite()
        && speed > 0.0
        && (speed - clip.speed).abs() > f64::EPSILON
    {
        return Some(InspectorAction::SetSpeed {
            clip_id: clip.id.clone(),
            speed,
        });
    }
    None
}

fn show_volume(ui: &mut egui::Ui, clip: &ClipViewModel, editable: bool) -> Option<InspectorAction> {
    let mut volume = clip.volume;
    if ui
        .add_enabled(
            editable && supports(EditCapability::Volume),
            DragValue::new(&mut volume)
                .speed(0.01)
                .range(0.0..=2.0)
                .prefix("Volume ")
                .suffix("×"),
        )
        .changed()
        && volume.is_finite()
        && (volume - clip.volume).abs() > f64::EPSILON
    {
        return Some(InspectorAction::SetVolume {
            clip_id: clip.id.clone(),
            volume,
        });
    }
    None
}

fn show_crop(ui: &mut egui::Ui, clip: &ClipViewModel, editable: bool) -> Option<InspectorAction> {
    if !supports(EditCapability::Crop) {
        return None;
    }
    let mut crop = clip.crop.clone().unwrap_or_else(identity_crop);
    let enabled = editable;
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(&mut crop.x)
                    .speed(0.01)
                    .range(0.0..=(1.0 - crop.width).max(0.001))
                    .prefix("X "),
            )
            .changed();
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(&mut crop.y)
                    .speed(0.01)
                    .range(0.0..=(1.0 - crop.height).max(0.001))
                    .prefix("Y "),
            )
            .changed();
    });
    ui.horizontal(|ui| {
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(&mut crop.width)
                    .speed(0.01)
                    .range(0.001..=(1.0 - crop.x).max(0.001))
                    .prefix("W "),
            )
            .changed();
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(&mut crop.height)
                    .speed(0.01)
                    .range(0.001..=(1.0 - crop.y).max(0.001))
                    .prefix("H "),
            )
            .changed();
    });
    if !crop_is_valid(&crop) {
        ui.label(
            RichText::new("Crop must stay inside normalized source bounds")
                .small()
                .color(Color32::from_rgb(255, 130, 130)),
        );
        return None;
    }
    if changed {
        return Some(InspectorAction::SetCrop {
            clip_id: clip.id.clone(),
            crop,
        });
    }
    if clip.crop.is_some()
        && ui
            .add_enabled(enabled, egui::Button::new("Reset crop"))
            .clicked()
    {
        return Some(InspectorAction::ClearCrop {
            clip_id: clip.id.clone(),
        });
    }
    None
}

fn show_transform(
    ui: &mut egui::Ui,
    clip: &ClipViewModel,
    editable: bool,
) -> Option<InspectorAction> {
    if !supports(EditCapability::Transform) {
        return None;
    }
    let mut transform = clip.transform.clone().unwrap_or_else(identity_transform);
    let supported = transform_is_supported(&transform);
    if !supported {
        ui.label(
            RichText::new("This transform contains unsupported 3D or anchor fields")
                .small()
                .color(Color32::from_rgb(255, 174, 92)),
        );
    }
    let enabled = editable && supported;
    let mut position_x = transform
        .x
        .or_else(|| transform_position_axis(&transform.position, "x"))
        .unwrap_or(0.0);
    let mut position_y = transform
        .y
        .or_else(|| transform_position_axis(&transform.position, "y"))
        .unwrap_or(0.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(&mut position_x).speed(0.01).prefix("X "),
            )
            .changed();
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(&mut position_y).speed(0.01).prefix("Y "),
            )
            .changed();
    });
    ui.horizontal(|ui| {
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(transform.scale.get_or_insert(1.0))
                    .speed(0.01)
                    .range(0.000001..=100.0)
                    .prefix("Scale "),
            )
            .changed();
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(transform.rotation.get_or_insert(0.0))
                    .speed(0.5)
                    .prefix("Rotation ")
                    .suffix("°"),
            )
            .changed();
    });
    ui.horizontal(|ui| {
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(transform.scale_x.get_or_insert(1.0))
                    .speed(0.01)
                    .range(0.000001..=100.0)
                    .prefix("Scale X "),
            )
            .changed();
        changed |= ui
            .add_enabled(
                enabled,
                DragValue::new(transform.scale_y.get_or_insert(1.0))
                    .speed(0.01)
                    .range(0.000001..=100.0)
                    .prefix("Scale Y "),
            )
            .changed();
    });
    if changed {
        transform.x = Some(position_x);
        transform.y = Some(position_y);
        return Some(InspectorAction::SetTransform {
            clip_id: clip.id.clone(),
            transform: Box::new(transform),
        });
    }
    if clip.transform.is_some()
        && ui
            .add_enabled(enabled, egui::Button::new("Reset transform"))
            .clicked()
    {
        return Some(InspectorAction::ClearTransform {
            clip_id: clip.id.clone(),
        });
    }
    None
}

fn show_opacity(
    ui: &mut egui::Ui,
    clip: &ClipViewModel,
    editable: bool,
) -> Option<InspectorAction> {
    let mut opacity = clip.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
    if ui
        .add_enabled(
            editable && supports(EditCapability::Opacity),
            DragValue::new(&mut opacity)
                .speed(0.01)
                .range(0.0..=1.0)
                .prefix("Opacity "),
        )
        .changed()
        && (clip.opacity.unwrap_or(1.0) - opacity).abs() > f64::EPSILON
    {
        return Some(InspectorAction::SetOpacity {
            clip_id: clip.id.clone(),
            opacity,
        });
    }
    None
}

fn show_effects(
    ui: &mut egui::Ui,
    track: &TrackViewModel,
    clip: &ClipViewModel,
    editable: bool,
) -> Option<InspectorAction> {
    if !supports(EditCapability::Effects) {
        return None;
    }
    let video_capable = matches!(track.kind.as_str(), "video" | "title");
    if !video_capable {
        ui.label(
            RichText::new("Color effects are supported only on video and title clips")
                .small()
                .color(Color32::from_gray(135)),
        );
        return None;
    }
    let mut effects = clip.effects.clone();
    let mut action = None;
    let mut remove_index = None;
    for (index, effect) in effects.iter_mut().enumerate() {
        match effect {
            Effect::Color(color) => {
                ui.label(RichText::new(format!("Color effect {}", index + 1)).small());
                let changed = show_color_params(ui, &mut color.params, editable);
                if changed {
                    action = Some(InspectorAction::SetEffects {
                        clip_id: clip.id.clone(),
                        effects: effects.clone(),
                    });
                    break;
                }
                if ui
                    .add_enabled(editable, egui::Button::new("Remove color effect"))
                    .clicked()
                {
                    remove_index = Some(index);
                    break;
                }
            }
            Effect::Generic(generic) => {
                ui.label(
                    RichText::new(format!(
                        "{} is not editable here; offline preview does not support it",
                        generic.kind
                    ))
                    .small()
                    .color(Color32::from_rgb(255, 174, 92)),
                );
            }
        }
    }
    if let Some(index) = remove_index {
        effects.remove(index);
        return Some(InspectorAction::SetEffects {
            clip_id: clip.id.clone(),
            effects,
        });
    }
    if action.is_some() {
        return action;
    }
    if ui
        .add_enabled(editable, egui::Button::new("Add color effect"))
        .clicked()
    {
        effects.push(Effect::Color(identity_color_effect()));
        return Some(InspectorAction::SetEffects {
            clip_id: clip.id.clone(),
            effects,
        });
    }
    if clip.effects.is_empty() {
        ui.label(
            RichText::new("No color effect")
                .small()
                .color(Color32::from_gray(135)),
        );
    }
    None
}

fn show_color_params(ui: &mut egui::Ui, params: &mut ColorEffectParams, editable: bool) -> bool {
    let mut changed = false;
    color_row(
        ui,
        editable,
        "Exposure",
        &mut params.exposure,
        -3.0..=3.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Contrast",
        &mut params.contrast,
        0.0..=4.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Brightness",
        &mut params.brightness,
        -1.0..=1.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Saturation",
        &mut params.saturation,
        0.0..=4.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Temperature",
        &mut params.temperature,
        -1000.0..=1000.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Tint",
        &mut params.tint,
        -1.0..=1.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Gamma",
        &mut params.gamma,
        0.1..=10.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Highlights",
        &mut params.highlights,
        -1.0..=1.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Shadows",
        &mut params.shadows,
        -1.0..=1.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Vibrance",
        &mut params.vibrance,
        -1.0..=1.0,
        &mut changed,
    );
    color_row(
        ui,
        editable,
        "Mix",
        &mut params.mix,
        0.0..=1.0,
        &mut changed,
    );
    changed
}

fn color_row(
    ui: &mut egui::Ui,
    editable: bool,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    changed: &mut bool,
) {
    *changed |= ui
        .add_enabled(
            editable,
            DragValue::new(value)
                .speed(0.01)
                .range(range)
                .prefix(format!("{label} ")),
        )
        .changed();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyframeProperty {
    Opacity,
    Scale,
    ScaleX,
    ScaleY,
    X,
    Y,
    Rotation,
}

impl KeyframeProperty {
    const ALL: [Self; 7] = [
        Self::Opacity,
        Self::Scale,
        Self::ScaleX,
        Self::ScaleY,
        Self::X,
        Self::Y,
        Self::Rotation,
    ];
    const OPACITY_ONLY: [Self; 1] = [Self::Opacity];

    const fn label(self) -> &'static str {
        match self {
            Self::Opacity => "Opacity",
            Self::Scale => "Scale",
            Self::ScaleX => "Scale X",
            Self::ScaleY => "Scale Y",
            Self::X => "Position X",
            Self::Y => "Position Y",
            Self::Rotation => "Rotation",
        }
    }

    const fn wire_name(self) -> &'static str {
        match self {
            Self::Opacity => "opacity",
            Self::Scale => "scale",
            Self::ScaleX => "scaleX",
            Self::ScaleY => "scaleY",
            Self::X => "x",
            Self::Y => "y",
            Self::Rotation => "rotation",
        }
    }

    fn from_wire(value: &str) -> Option<Self> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '_', '-'], "")
            .as_str()
        {
            "opacity" => Some(Self::Opacity),
            "scale" | "transform.scale" | "scaleuniform" => Some(Self::Scale),
            "scalex" | "transform.scalex" => Some(Self::ScaleX),
            "scaley" | "transform.scaley" => Some(Self::ScaleY),
            "x" | "position.x" | "transform.x" | "transform.position.x" => Some(Self::X),
            "y" | "position.y" | "transform.y" | "transform.position.y" => Some(Self::Y),
            "rotation" | "transform.rotation" => Some(Self::Rotation),
            _ => None,
        }
    }
}

fn show_keyframes(
    ui: &mut egui::Ui,
    clip: &ClipViewModel,
    grid: FrameGrid,
    editable: bool,
    opacity_only: bool,
) -> Option<InspectorAction> {
    if !supports(EditCapability::Keyframes) {
        return None;
    }
    let mut keyframes = clip.keyframes.clone();
    let allowed_properties = if opacity_only {
        &KeyframeProperty::OPACITY_ONLY[..]
    } else {
        &KeyframeProperty::ALL[..]
    };
    let mut remove_index = None;
    for (index, keyframe) in keyframes.iter_mut().enumerate() {
        let Some(mut property) = KeyframeProperty::from_wire(&keyframe.property) else {
            ui.label(
                RichText::new(format!("Unsupported property: {}", keyframe.property))
                    .small()
                    .color(Color32::from_rgb(255, 174, 92)),
            );
            if ui
                .add_enabled(editable, egui::Button::new("Remove keyframe"))
                .clicked()
            {
                remove_index = Some(index);
                break;
            }
            continue;
        };
        let Some(mut value) = json_number(&keyframe.value) else {
            ui.label(
                RichText::new("Non-numeric keyframe values are unavailable in this editor")
                    .small()
                    .color(Color32::from_rgb(255, 174, 92)),
            );
            continue;
        };
        let mut time = grid.frame_at(keyframe.time).max(0);
        let original_property = property;
        let original_time = time;
        let original_value = value;
        let initially_supported = allowed_properties.contains(&property);
        let linear_keyframe = keyframe
            .easing
            .as_deref()
            .is_none_or(|easing| easing == "linear");
        let keyframe_editable = editable && linear_keyframe && initially_supported;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(editable && linear_keyframe, |ui| {
                ComboBox::from_id_salt(("keyframe-property", &clip.id, index))
                    .selected_text(property.label())
                    .show_ui(ui, |ui| {
                        for candidate in allowed_properties {
                            ui.selectable_value(&mut property, *candidate, candidate.label());
                        }
                    });
            });
            ui.add_enabled(
                keyframe_editable,
                DragValue::new(&mut time)
                    .speed(1)
                    .range(0..=grid.frame_at(clip.timeline.duration()).max(1))
                    .suffix(" fr"),
            );
            let (min, max) = keyframe_value_range(property);
            ui.add_enabled(
                keyframe_editable,
                DragValue::new(&mut value).speed(0.01).range(min..=max),
            );
            if ui
                .add_enabled(keyframe_editable, egui::Button::new("Remove"))
                .clicked()
            {
                remove_index = Some(index);
            }
        });
        let property_supported = allowed_properties.contains(&property);
        let keyframe_editable = editable && linear_keyframe && property_supported;
        if !keyframe_editable {
            ui.label(
                RichText::new(if !property_supported {
                    "This track supports opacity keyframes only"
                } else {
                    "Only linear keyframes are editable"
                })
                .small()
                .color(Color32::from_rgb(255, 174, 92)),
            );
        }
        if keyframe_editable
            && (property != original_property || time != original_time || value != original_value)
        {
            keyframe.property = property.wire_name().into();
            keyframe.time = grid.seconds_at(time);
            keyframe.value = number_value(value);
            return Some(InspectorAction::SetKeyframes {
                clip_id: clip.id.clone(),
                keyframes,
            });
        }
        if remove_index.is_some() {
            break;
        }
    }
    if let Some(index) = remove_index {
        keyframes.remove(index);
        return Some(InspectorAction::SetKeyframes {
            clip_id: clip.id.clone(),
            keyframes,
        });
    }
    if ui
        .add_enabled(editable, egui::Button::new("Add keyframe"))
        .clicked()
    {
        keyframes.push(default_keyframe(
            grid.seconds_at(0),
            clip.opacity.unwrap_or(1.0),
            keyframes.len(),
        ));
        return Some(InspectorAction::SetKeyframes {
            clip_id: clip.id.clone(),
            keyframes,
        });
    }
    if clip.keyframes.is_empty() {
        ui.label(
            RichText::new(
                "No keyframes · opacity, scale, position, and rotation support linear values",
            )
            .small()
            .color(Color32::from_gray(135)),
        );
    }
    None
}

fn keyframe_value_range(property: KeyframeProperty) -> (f64, f64) {
    match property {
        KeyframeProperty::Opacity => (0.0, 1.0),
        KeyframeProperty::Scale | KeyframeProperty::ScaleX | KeyframeProperty::ScaleY => {
            (0.000001, 100.0)
        }
        KeyframeProperty::X | KeyframeProperty::Y | KeyframeProperty::Rotation => {
            (-1_000_000.0, 1_000_000.0)
        }
    }
}

fn show_transition(
    ui: &mut egui::Ui,
    track: &TrackViewModel,
    clip: &ClipViewModel,
    editable: bool,
) -> Option<InspectorAction> {
    if !supports(EditCapability::Transitions) {
        return None;
    }
    let Some(overlap) = transition_overlap(track, &clip.id) else {
        ui.label(
            RichText::new("Transitions require overlapping adjacent clips")
                .small()
                .color(Color32::from_gray(135)),
        );
        return None;
    };
    let duration = overlap;
    ui.label(
        RichText::new(format!(
            "Overlap {:.3}s · transition duration is derived from the overlap",
            overlap
        ))
        .small()
        .color(Color32::from_gray(135)),
    );
    let mut transitions = clip.transitions.clone();
    if transitions.is_empty() {
        ui.horizontal(|ui| {
            for kind in ["fade", "crossfade", "dissolve"] {
                if ui.add_enabled(editable, egui::Button::new(kind)).clicked() {
                    transitions.push(new_transition(kind, duration));
                }
            }
        });
        if !transitions.is_empty() {
            return Some(InspectorAction::SetTransitions {
                clip_id: clip.id.clone(),
                transitions,
            });
        }
        ui.label(
            RichText::new("Supported types: fade, crossfade, dissolve")
                .small()
                .color(Color32::from_gray(135)),
        );
        return None;
    }

    let current = transitions.first_mut()?;
    let Some(mut kind) = transition_kind(&current.kind) else {
        ui.label(
            RichText::new(format!("Unsupported transition: {}", current.kind))
                .small()
                .color(Color32::from_rgb(255, 174, 92)),
        );
        if ui
            .add_enabled(editable, egui::Button::new("Remove transition"))
            .clicked()
        {
            transitions.clear();
            return Some(InspectorAction::SetTransitions {
                clip_id: clip.id.clone(),
                transitions,
            });
        }
        return None;
    };
    let original = kind;
    ui.add_enabled_ui(editable, |ui| {
        ComboBox::from_id_salt(("transition", &clip.id))
            .selected_text(kind)
            .show_ui(ui, |ui| {
                for candidate in ["fade", "crossfade", "dissolve"] {
                    ui.selectable_value(&mut kind, candidate, candidate);
                }
            });
    });
    if (current.duration - duration).abs() > 1e-6 {
        ui.label(
            RichText::new(format!(
                "Existing duration {:.3}s must match the {:.3}s overlap",
                current.duration, duration
            ))
            .small()
            .color(Color32::from_rgb(255, 174, 92)),
        );
        if ui
            .add_enabled(editable, egui::Button::new("Align duration"))
            .clicked()
        {
            current.duration = duration;
            return Some(InspectorAction::SetTransitions {
                clip_id: clip.id.clone(),
                transitions,
            });
        }
    }
    if kind != original {
        current.kind = kind.to_owned();
        current.duration = duration;
        return Some(InspectorAction::SetTransitions {
            clip_id: clip.id.clone(),
            transitions,
        });
    }
    if ui
        .add_enabled(editable, egui::Button::new("Remove transition"))
        .clicked()
    {
        transitions.clear();
        return Some(InspectorAction::SetTransitions {
            clip_id: clip.id.clone(),
            transitions,
        });
    }
    None
}

fn capability_note(ui: &mut egui::Ui, capability_kind: EditCapability) {
    let entry = capability(capability_kind);
    let text = match entry.offline {
        CapabilitySupport::Unsupported => format!(
            "Unavailable: offline export does not support this {}",
            capability_label(capability_kind)
        ),
        CapabilitySupport::Limited if !entry.live.is_exact() => format!(
            "Rendered preview · supported export subset. {}",
            entry.offline_note
        ),
        CapabilitySupport::Limited => format!("Supported export subset. {}", entry.offline_note),
        CapabilitySupport::Exact if !entry.live.is_exact() => {
            format!("Rendered preview required. {}", entry.live_note)
        }
        CapabilitySupport::Exact => String::new(),
    };
    if !text.is_empty() {
        ui.label(RichText::new(text).small().color(Color32::from_gray(135)));
    }
}

fn supports(capability_kind: EditCapability) -> bool {
    capability(capability_kind).offline != CapabilitySupport::Unsupported
}

fn capability_label(capability_kind: EditCapability) -> &'static str {
    match capability_kind {
        EditCapability::Placement => "placement",
        EditCapability::SourceRange => "source range",
        EditCapability::Speed => "speed",
        EditCapability::Volume => "volume",
        EditCapability::Captions => "captions",
        EditCapability::Crop => "crop",
        EditCapability::Transform => "transform",
        EditCapability::Opacity => "opacity",
        EditCapability::LayeredVideo => "layered video",
        EditCapability::AudioMixing => "audio mixing",
        EditCapability::Effects => "effects",
        EditCapability::Keyframes => "keyframes",
        EditCapability::Transitions => "transitions",
        EditCapability::Overlays => "overlays",
        EditCapability::Titles => "titles",
    }
}

fn visual_track(track: &TrackViewModel) -> bool {
    matches!(track.kind.as_str(), "video" | "title")
}

fn crop_is_valid(crop: &CropRegion) -> bool {
    [crop.x, crop.y, crop.width, crop.height]
        .into_iter()
        .all(|value| value.is_finite())
        && crop.x >= 0.0
        && crop.y >= 0.0
        && crop.width > 0.0
        && crop.height > 0.0
        && crop.x + crop.width <= 1.0 + f64::EPSILON
        && crop.y + crop.height <= 1.0 + f64::EPSILON
}

fn transform_is_supported(transform: &Transform) -> bool {
    transform.z.is_none()
        && transform.rotation_x.is_none()
        && transform.rotation_y.is_none()
        && transform.anchor_x.is_none()
        && transform.anchor_y.is_none()
        && transform.anchor.is_none()
        && transform.position.as_ref().is_none_or(
            |position| matches!(position, JsonValue::Object(values) if !values.contains_key("z")),
        )
}

fn transform_position_axis(position: &Option<JsonValue>, axis: &str) -> Option<f64> {
    let JsonValue::Object(position) = position.as_ref()? else {
        return None;
    };
    match position.get(axis)? {
        JsonValue::Number(value) => value.as_f64().filter(|value| value.is_finite()),
        _ => None,
    }
}

fn identity_crop() -> CropRegion {
    serde_json::from_value(json!({
        "x": 0.0,
        "y": 0.0,
        "width": 1.0,
        "height": 1.0
    }))
    .expect("identity crop is valid")
}

fn identity_transform() -> Transform {
    serde_json::from_value(json!({
        "x": 0.0,
        "y": 0.0,
        "scale": 1.0,
        "rotation": 0.0
    }))
    .expect("identity transform is valid")
}

fn identity_color_effect() -> ColorEffect {
    serde_json::from_value(json!({
        "type": "color",
        "params": {
            "exposure": 0.0,
            "contrast": 1.0,
            "brightness": 0.0,
            "saturation": 1.0,
            "temperature": 0.0,
            "tint": 0.0,
            "gamma": 1.0,
            "highlights": 0.0,
            "shadows": 0.0,
            "vibrance": 0.0,
            "mix": 1.0
        }
    }))
    .expect("identity color effect is valid")
}

fn json_number(value: &JsonValue) -> Option<f64> {
    match value {
        JsonValue::Number(number) => number.as_f64().filter(|value| value.is_finite()),
        _ => None,
    }
}

fn number_value(value: f64) -> JsonValue {
    JsonValue::Number(Number::from_f64(value).expect("inspector only emits finite numbers"))
}

fn default_keyframe(time: f64, value: f64, index: usize) -> Keyframe {
    serde_json::from_value(json!({
        "id": format!("keyframe-{index}"),
        "property": "opacity",
        "time": time,
        "value": value,
        "easing": "linear"
    }))
    .expect("default keyframe is valid")
}

fn transition_kind(kind: &str) -> Option<&'static str> {
    match kind.to_ascii_lowercase().as_str() {
        "fade" => Some("fade"),
        "crossfade" => Some("crossfade"),
        "dissolve" => Some("dissolve"),
        _ => None,
    }
}

fn new_transition(kind: &str, duration: f64) -> Transition {
    serde_json::from_value(json!({
        "type": kind,
        "duration": duration,
        "alignment": "center"
    }))
    .expect("supported transition is valid")
}

fn transition_overlap(track: &TrackViewModel, clip_id: &str) -> Option<f64> {
    let index = track.clips.iter().position(|clip| clip.id == clip_id)?;
    let mut overlaps = Vec::new();
    if let (Some(previous), Some(current)) = (
        index.checked_sub(1).and_then(|i| track.clips.get(i)),
        track.clips.get(index),
    ) {
        overlaps.push(previous.timeline.end - current.timeline.start);
    }
    if let (Some(current), Some(next)) = (track.clips.get(index), track.clips.get(index + 1)) {
        overlaps.push(current.timeline.end - next.timeline.start);
    }
    overlaps
        .into_iter()
        .filter(|overlap| overlap.is_finite() && *overlap > 0.0)
        .max_by(f64::total_cmp)
}

#[cfg(test)]
mod tests {
    use super::{
        crop_is_valid, default_keyframe, identity_crop, identity_transform, new_transition,
        transition_overlap, KeyframeProperty,
    };
    use crate::timeline::{ClipViewModel, MediaType, TimelineRange, TrackViewModel};
    use cut_core::FrameGrid;
    use cut_domain::JsonValue;
    use serde_json::json;

    fn clip(id: &str, start: f64, end: f64) -> ClipViewModel {
        ClipViewModel {
            id: id.into(),
            source_id: None,
            source_label: id.into(),
            media_type: MediaType::Video,
            timeline: TimelineRange::new(start, end).unwrap(),
            source: None,
            source_duration: None,
            speed: 1.0,
            volume: 1.0,
            opacity: None,
            crop: None,
            transform: None,
            effects: Vec::new(),
            keyframes: Vec::new(),
            transitions: Vec::new(),
            thumbnail_cache_key: None,
            waveform_cache_key: None,
        }
    }

    #[test]
    fn editor_defaults_are_valid_typed_values() {
        assert!(crop_is_valid(&identity_crop()));
        assert_eq!(identity_transform().scale, Some(1.0));
        assert_eq!(new_transition("fade", 0.5).kind, "fade");
        let keyframe = default_keyframe(0.0, 1.0, 2);
        assert!(matches!(keyframe.value, JsonValue::Number(_)));
    }

    #[test]
    fn transition_overlap_comes_from_adjacent_clip_edges() {
        let track = TrackViewModel {
            id: "video-1".into(),
            index: 0,
            name: "Video".into(),
            kind: "video".into(),
            clip_count: 2,
            first_clip_id: Some("a".into()),
            first_clip_volume: 1.0,
            enabled: true,
            muted: false,
            locked: false,
            clips: vec![clip("a", 0.0, 2.0), clip("b", 1.5, 3.0)],
        };
        assert_eq!(transition_overlap(&track, "a"), Some(0.5));
        assert_eq!(transition_overlap(&track, "b"), Some(0.5));
    }

    #[test]
    fn keyframes_use_the_project_grid() {
        let grid = FrameGrid::new(29.97).unwrap();
        assert_eq!(grid.seconds_at(grid.frame_at(1.0)), 30.0 / 29.97);
        assert_eq!(
            KeyframeProperty::from_wire("transform.x")
                .unwrap()
                .wire_name(),
            "x"
        );
        let _ = json!(grid.fps());
    }
}
