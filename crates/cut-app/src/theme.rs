use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, Visuals};

pub const LINE: Color32 = Color32::from_rgb(36, 36, 36);
pub const MUTED: Color32 = Color32::from_rgb(133, 133, 133);
pub const LIME: Color32 = Color32::from_rgb(199, 255, 86);

/// Apply the selected Phase 0 timeline-first visual language to every surface.
pub fn apply(context: &egui::Context) {
    let mut style = (*context.style_of(Theme::Dark)).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(9.0, 5.0);
    style.visuals = Visuals::dark();
    style.visuals.panel_fill = Color32::BLACK;
    style.visuals.window_fill = Color32::BLACK;
    style.visuals.extreme_bg_color = Color32::from_rgb(5, 5, 5);
    style.visuals.faint_bg_color = Color32::from_rgb(12, 12, 12);
    style.visuals.widgets.noninteractive.bg_fill = Color32::BLACK;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(10, 10, 10);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(24, 24, 24);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(70));
    style.visuals.widgets.active.bg_fill = Color32::WHITE;
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::BLACK);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::ZERO;
    style.visuals.widgets.hovered.corner_radius = CornerRadius::ZERO;
    style.visuals.widgets.active.corner_radius = CornerRadius::ZERO;
    style.visuals.selection.bg_fill = Color32::from_rgb(18, 51, 58);
    style.visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(98, 227, 255));
    style.visuals.window_corner_radius = CornerRadius::ZERO;
    style.visuals.window_stroke = Stroke::new(1.0, LINE);
    style.visuals.override_text_color = Some(Color32::WHITE);
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(15.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(12.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(11.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Small, FontId::new(10.0, FontFamily::Monospace));
    context.set_style_of(Theme::Dark, style);
}
