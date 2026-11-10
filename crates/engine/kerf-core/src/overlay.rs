//! Overlay filter helpers.

/// `value` rounded to 6 decimals, without trailing zeros.
pub fn fmt(value: f64) -> String {
    let s = format!("{:.6}", value);
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
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
