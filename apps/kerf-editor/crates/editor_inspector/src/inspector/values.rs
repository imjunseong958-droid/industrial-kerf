//! Numbers, colours and words as the inspector shows and reads them: a
//! slider's range and places, a number typed, a colour in kerf's form, and
//! a parameter's name in words.

use super::catalog::Param;
use gpui::Rgba;
use serde_json::Value;

/// Words for the properties whose names say too little.
const LABELS: &[(&str, &str)] = &[
    ("x", "Position X"),
    ("y", "Position Y"),
    ("z", "Distance"),
    ("scale_x", "Scale X"),
    ("scale_y", "Scale Y"),
    ("anchor_x", "Anchor X"),
    ("anchor_y", "Anchor Y"),
    ("crop_l", "Crop Left"),
    ("crop_r", "Crop Right"),
    ("crop_t", "Crop Top"),
    ("crop_b", "Crop Bottom"),
    ("three_d", "3D"),
    ("rotate_x", "Tilt"),
    ("rotate_y", "Turn"),
    ("bevel_profile", "Bevel Shape"),
    ("along", "Along Path"),
    ("orient", "Face Path"),
    ("reveal_soft", "Softness"),
    ("wght", "Weight Axis"),
    ("wdth", "Width Axis"),
    ("slnt", "Slant Axis"),
    ("opsz", "Optical Size"),
    ("ital", "Italic Axis"),
];

/// The range a clip property's slider covers, where the catalog's is far
/// wider than a hand moves through (a typed value may still go to the
/// catalog's limits).
const SOFT: &[(&str, f64, f64)] = &[
    ("x", -1.0, 1.0),
    ("y", -1.0, 1.0),
    ("z", -10.0, 10.0),
    ("depth", 0.0, 1.0),
    ("bevel", 0.0, 0.2),
    ("scale", 0.0, 4.0),
    ("scale_x", -4.0, 4.0),
    ("scale_y", -4.0, 4.0),
    ("rotation", -180.0, 180.0),
    ("rotate_x", -180.0, 180.0),
    ("rotate_y", -180.0, 180.0),
    ("anchor_x", -1.0, 1.0),
    ("anchor_y", -1.0, 1.0),
    ("gain", -48.0, 12.0),
    ("from_x", -1.0, 1.0),
    ("from_y", -1.0, 1.0),
    ("from_scale", 0.0, 4.0),
    ("from_rotation", -180.0, 180.0),
    ("reveal_soft", 0.01, 20.0),
];

/// The range a number's slider covers: the catalog's, narrowed where it is
/// far wider than a hand moves through, and widened to hold `value`.
pub(super) fn soft_range(param: &Param, clip_level: bool, value: f64) -> (f64, f64) {
    let named = clip_level
        .then(|| SOFT.iter().find(|(name, ..)| *name == param.name))
        .flatten()
        .map(|(_, lo, hi)| (*lo, *hi));
    let (mut lo, mut hi) = named.unwrap_or_else(|| guessed_range(param));
    let (min, max) = param.hard();
    lo = lo.max(min);
    hi = hi.min(max);
    if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
        (lo, hi) = guessed_range(param);
    }
    if value.is_finite() {
        lo = lo.min(value);
        hi = hi.max(value);
    }
    (lo, hi)
}

fn guessed_range(param: &Param) -> (f64, f64) {
    let default = param.default.as_f64().unwrap_or(0.0);
    let unit = param.unit.as_deref().unwrap_or_default();
    match (param.min, param.max) {
        (Some(min), Some(max)) if max > min => {
            let span = max - min;
            if unit == "degrees" && span > 720.0 {
                if min < 0.0 {
                    (-180.0, 180.0)
                } else {
                    (0.0, 360.0)
                }
            } else if (unit.starts_with("frame ") || unit.starts_with("clip ")) && span > 4.0 {
                if min < 0.0 {
                    (-1.0, 1.0)
                } else {
                    (min, min + 1.0)
                }
            } else if unit == "×" && span > 10.0 {
                (if min < 0.0 { -4.0 } else { min }, 4.0)
            } else if unit == "px" && span > 200.0 {
                (min, min + 100.0)
            } else {
                (min, max)
            }
        }
        (Some(min), None) => (min, min + (default - min).abs().max(1.0) * 4.0),
        (None, Some(max)) => (max - (max - default).abs().max(1.0) * 4.0, max),
        _ => {
            let reach = default.abs().max(1.0) * 2.0;
            (default - reach, default + reach)
        }
    }
}

/// The places a number over a range `span` wide is shown and moved in.
pub(super) fn decimals(span: f64) -> usize {
    if span <= 1.5 {
        3
    } else if span <= 10.0 {
        2
    } else if span <= 200.0 {
        1
    } else {
        0
    }
}

pub(super) fn round_to(value: f64, decimals: usize) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (value * scale).round() / scale
}

/// `value` with `decimals` places, more (up to four) when it has them.
pub(super) fn format_number(value: f64, decimals: usize) -> String {
    let most = decimals.max(4);
    let places = (decimals..=most)
        .find(|places| (round_to(value, *places) - value).abs() < 1e-9)
        .unwrap_or(most);
    let mut shown = format!("{value:.places$}");
    // Past `decimals`, a zero at the end says nothing.
    if places > decimals {
        while shown.ends_with('0')
            && shown
                .split_once('.')
                .is_some_and(|(_, after)| after.len() > decimals)
        {
            shown.pop();
        }
        if shown.ends_with('.') {
            shown.pop();
        }
    }
    // No minus before a zero.
    if shown.starts_with('-') && shown[1..].chars().all(|c| c == '0' || c == '.') {
        shown.remove(0);
    }
    shown
}

/// A number as typed: a unit after it is let be, and a comma may be the point.
pub(super) fn parse_number(typed: &str) -> Option<f64> {
    let typed = typed.trim().trim_start_matches('+');
    let typed = typed.trim_end_matches(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','));
    let typed = if typed.contains('.') {
        typed.to_owned()
    } else {
        typed.replace(',', ".")
    };
    typed
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// What follows a number of `unit`, where it says something short.
pub(super) fn unit_suffix(unit: Option<&str>) -> &'static str {
    match unit.unwrap_or_default() {
        "degrees" => "°",
        "×" => "×",
        "dB" | "dBFS" => "dB",
        "px" => "px",
        "ms" => "ms",
        "Hz" => "Hz",
        "K" => "K",
        "%" => "%",
        _ => "",
    }
}

/// A held value in words.
pub(super) fn value_words(param: &Param, value: &Value, clip_level: bool) -> String {
    match value {
        Value::Number(number) => {
            let number = number.as_f64().unwrap_or_default();
            let (lo, hi) = soft_range(param, clip_level, number);
            format!(
                "{}{}",
                format_number(number, decimals(hi - lo)),
                unit_suffix(param.unit.as_deref())
            )
        }
        Value::Bool(on) => if *on { "On" } else { "Off" }.to_owned(),
        Value::String(words) => words.clone(),
        Value::Null => "—".to_owned(),
        other => other.to_string(),
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` may be left out) as its four channels.
pub(super) fn channels(text: &str) -> Option<[u8; 4]> {
    let digits = text.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let pair = |index: usize| u8::from_str_radix(&digits[index..index + 2], 16).ok();
    let single = |index: usize| {
        u8::from_str_radix(&digits[index..index + 1], 16)
            .ok()
            .map(|v| v * 17)
    };
    Some(match digits.len() {
        3 => [single(0)?, single(1)?, single(2)?, 255],
        4 => [single(0)?, single(1)?, single(2)?, single(3)?],
        6 => [pair(0)?, pair(2)?, pair(4)?, 255],
        8 => [pair(0)?, pair(2)?, pair(4)?, pair(6)?],
        _ => return None,
    })
}

/// A colour as kerf writes it: `#rrggbb`, with its alpha only when not opaque.
pub(super) fn hex(r: u8, g: u8, b: u8, alpha: u8) -> String {
    if alpha == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{alpha:02x}")
    }
}

/// A typed colour in kerf's form, or None when it is no colour.
pub(super) fn parse_hex(text: &str) -> Option<String> {
    channels(text).map(|[r, g, b, alpha]| hex(r, g, b, alpha))
}

pub(super) fn swatch_color(text: &str) -> Rgba {
    let [r, g, b, alpha] = channels(text).unwrap_or([0, 0, 0, 255]);
    Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: f32::from(alpha) / 255.0,
    }
}

/// A parameter's or generator's name in words: `scale_x` is "Scale X".
pub(super) fn humanize(name: &str) -> String {
    if let Some((_, words)) = LABELS.iter().find(|(known, _)| *known == name) {
        return (*words).to_owned();
    }
    words_of(name)
}

/// A name in words as it is spelt, with none of the properties' own
/// words: a choice's option (`x` is "X", not "Position X").
pub(super) fn words_of(name: &str) -> String {
    name.split(['_', '.', '-'])
        .filter(|word| !word.is_empty())
        .enumerate()
        .map(|(index, word)| match word {
            "db" => "dB".to_owned(),
            "dbfs" => "dBFS".to_owned(),
            "hz" => "Hz".to_owned(),
            "x" | "y" | "z" | "id" | "rgb" => word.to_uppercase(),
            _ if index == 0 => {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
            _ => word.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a blurb says no more than a name: "Gaussian blur." under "Gaussian blur".
pub(super) fn same_words(blurb: &str, name: &str) -> bool {
    let bare = |words: &str| words.trim().trim_end_matches('.').to_lowercase();
    bare(blurb).is_empty() || bare(blurb) == bare(name)
}
