//! Overlay filter helpers.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct CaptionCue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct CaptionStyle {
    pub font_size: Option<f64>,
    pub font_color: Option<String>,
    pub outline_color: Option<String>,
    pub position: Option<String>,
    pub margin_v: Option<f64>,
    pub outline_width: Option<f64>,
    pub background_color: Option<String>,
    pub font_family: Option<String>,
}

/// `String(Number(n.toFixed(6)))`. Round to 6 decimals, then trim trailing zeros.
pub fn fmt(value: f64) -> String {
    let s = format!("{:.6}", value);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

/// Escape a file path for use inside an ffmpeg filter argument.
pub fn escape_filter_path(path: &str) -> String {
    path.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// Escape text for drawtext's filter value.
pub fn escape_drawtext_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace("\r\n", "\\n")
        .replace(['\r', '\n'], "\\n")
        .replace('\'', "\\'")
        .replace(':', "\\\\:")
        .replace(',', "\\,")
        .replace(';', "\\;")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('%', "\\\\%")
        .replace(' ', "\\ ")
}

/// FFmpeg atempo accepts [0.5, 100]. Split extreme speeds into a product chain.
pub fn atempo_filters(speed: f64) -> Vec<String> {
    if !(speed.is_finite() && speed > 0.0) {
        return Vec::new();
    }
    let mut remaining = speed;
    let mut filters = Vec::new();
    while remaining > 100.0 {
        filters.push("atempo=100".to_string());
        remaining /= 100.0;
    }
    while remaining < 0.5 {
        filters.push("atempo=0.5".to_string());
        remaining /= 0.5;
    }
    if (remaining - 1.0).abs() > 1e-9 {
        let clamped = remaining.clamp(0.5, 100.0);
        filters.push(format!("atempo={}", fmt(clamped)));
    }
    filters
}

fn to_ass_color(color: &str) -> String {
    if let Some(hex) = color.strip_prefix('#') {
        if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let r = &hex[0..2];
            let g = &hex[2..4];
            let b = &hex[4..6];
            return format!("&H00{b}{g}{r}").to_uppercase();
        }
    }
    color.to_string()
}

fn alignment(position: &str) -> Option<i32> {
    match position {
        "bottom" => Some(2),
        "center" => Some(5),
        "top" => Some(8),
        _ => None,
    }
}

/// Build the `subtitles=` filter string for one overlay entry.
pub fn subtitles_filter(path: &str, style: Option<&Value>) -> String {
    let mut style_parts: Vec<String> = Vec::new();
    if let Some(style) = style.and_then(|v| v.as_object()) {
        if let Some(font_size) = style.get("fontSize").and_then(|v| v.as_f64()) {
            style_parts.push(format!("FontSize={font_size}"));
        }
        if let Some(font_color) = style.get("fontColor").and_then(|v| v.as_str()) {
            style_parts.push(format!("PrimaryColour={}", to_ass_color(font_color)));
        }
        if let Some(outline_color) = style.get("outlineColor").and_then(|v| v.as_str()) {
            style_parts.push(format!("OutlineColour={}", to_ass_color(outline_color)));
        }
        if let Some(position) = style.get("position").and_then(|v| v.as_str()) {
            if let Some(align) = alignment(position) {
                style_parts.push(format!("Alignment={align}"));
            }
        }
        if let Some(margin_v) = style.get("marginV").and_then(|v| v.as_f64()) {
            style_parts.push(format!("MarginV={margin_v}"));
        }
    }
    let force_style = if style_parts.is_empty() {
        String::new()
    } else {
        format!(":force_style='{}'", style_parts.join(","))
    };
    format!(
        "subtitles=filename='{}'{force_style}",
        escape_filter_path(path)
    )
}

fn escape_drawtext_value(value: &str) -> String {
    escape_drawtext_text(value)
}

fn drawtext_color(value: &str) -> String {
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!("0x{hex}");
        }
        if hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!("0x{hex}");
        }
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return value.to_string();
    }
    escape_drawtext_value(value)
}

fn style_number(value: Option<&Value>) -> Option<f64> {
    value.and_then(|v| v.as_f64()).filter(|n| n.is_finite())
}

fn caption_style_from_value(style: Option<&Value>) -> CaptionStyle {
    let Some(style) = style.and_then(|v| v.as_object()) else {
        return CaptionStyle::default();
    };
    CaptionStyle {
        font_size: style_number(style.get("fontSize")),
        font_color: style
            .get("fontColor")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        outline_color: style
            .get("outlineColor")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        position: style
            .get("position")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        margin_v: style_number(style.get("marginV")),
        outline_width: style_number(style.get("outlineWidth")),
        background_color: style
            .get("backgroundColor")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        font_family: style
            .get("fontFamily")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

/// Build one timed drawtext filter for a caption cue.
pub fn captions_filter(cue: &CaptionCue, style: Option<&Value>) -> String {
    let parsed = caption_style_from_value(style);
    let position = parsed.position.as_deref().unwrap_or("bottom");
    let margin = parsed.margin_v.unwrap_or(24.0).max(0.0);
    let y = if position == "top" {
        fmt(margin)
    } else if position == "center" {
        "(h-text_h)/2".to_string()
    } else {
        format!("h-text_h-{}", fmt(margin))
    };

    let mut options = vec![
        format!("text={}", escape_drawtext_text(&cue.text)),
        format!("enable='between(t,{},{})'", fmt(cue.start), fmt(cue.end)),
        "expansion=none".to_string(),
        "x='(w-text_w)/2'".to_string(),
        format!("y='{y}'"),
    ];

    if let Some(font_size) = parsed.font_size {
        if font_size > 0.0 {
            options.push(format!("fontsize={}", fmt(font_size)));
        }
    }
    if let Some(ref font_color) = parsed.font_color {
        options.push(format!("fontcolor={}", drawtext_color(font_color)));
    }
    if let Some(ref outline_color) = parsed.outline_color {
        options.push(format!("bordercolor={}", drawtext_color(outline_color)));
        let outline_width = parsed.outline_width.unwrap_or(2.0);
        if outline_width > 0.0 {
            options.push(format!("borderw={}", fmt(outline_width)));
        }
    }
    if let Some(ref background_color) = parsed.background_color {
        options.push("box=1".to_string());
        options.push(format!("boxcolor={}", drawtext_color(background_color)));
    }
    if let Some(ref font_family) = parsed.font_family {
        if !font_family.is_empty() {
            options.push(format!("font={}", escape_drawtext_value(font_family)));
        }
    }

    format!("drawtext={}", options.join(":"))
}
