use crate::PlaybackError;
use ffmpeg_next as ffmpeg;
use std::path::Path;
use std::sync::OnceLock;

pub(crate) fn ensure_init() -> Result<(), PlaybackError> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    match INIT.get_or_init(|| {
        ffmpeg::init()
            .map(|()| {
                ffmpeg::log::set_level(ffmpeg::log::Level::Error);
            })
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => Ok(()),
        Err(error) => Err(PlaybackError::Ffmpeg(error.clone())),
    }
}

pub(crate) fn map_error(error: ffmpeg::Error) -> PlaybackError {
    PlaybackError::Ffmpeg(error.to_string())
}

pub(crate) fn is_again(error: &ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other { errno } if *errno == ffmpeg::util::error::EAGAIN)
}

pub(crate) fn is_eof(error: &ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Eof)
}

pub(crate) fn seek_to(input: &mut ffmpeg::format::context::Input, seconds: f64) {
    if !seconds.is_finite() || seconds <= 0.0 {
        return;
    }
    let ts = (seconds * 1_000_000.0).round() as i64;
    let _ = input.seek(ts, ..ts);
}

pub(crate) fn seconds(ts: i64, time_base: ffmpeg::Rational) -> f64 {
    let den = time_base.1;
    if den == 0 {
        return 0.0;
    }
    ts as f64 * f64::from(time_base.0) / f64::from(den)
}

pub(crate) fn format_number(value: f64) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    format!("{value:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

pub(crate) fn escape_filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

pub(crate) fn packed_rgba(
    frame: &ffmpeg::frame::Video,
    width: u32,
    height: u32,
) -> Result<std::sync::Arc<[u8]>, PlaybackError> {
    let byte_len = frame_byte_len(width, height).ok_or_else(|| {
        PlaybackError::Ffmpeg(format!(
            "decoded frame dimensions are too large: {width}x{height}"
        ))
    })?;
    let src_width = frame.width();
    let src_height = frame.height();
    if src_width != width || src_height != height {
        return Err(PlaybackError::Ffmpeg(format!(
            "filter output size {src_width}x{src_height} != {width}x{height}"
        )));
    }
    let stride = frame.stride(0);
    let row_bytes = width as usize * 4;
    if stride < row_bytes {
        return Err(PlaybackError::Ffmpeg(format!(
            "RGBA stride {stride} is smaller than row width {row_bytes}"
        )));
    }
    let data = frame.data(0);
    let mut pixels = vec![0_u8; byte_len];
    for y in 0..height as usize {
        let src = y * stride;
        let dst = y * row_bytes;
        pixels[dst..dst + row_bytes].copy_from_slice(&data[src..src + row_bytes]);
    }
    Ok(std::sync::Arc::from(pixels.into_boxed_slice()))
}

pub(crate) fn packed_f32(frame: &ffmpeg::frame::Audio) -> Result<Vec<f32>, PlaybackError> {
    let channels = frame.channels() as usize;
    let samples = frame.samples();
    let count = samples.saturating_mul(channels.max(1));
    let data = frame.data(0);
    if data.len() < count * 4 {
        return Err(PlaybackError::Ffmpeg(format!(
            "audio frame is truncated: {} bytes for {count} samples",
            data.len()
        )));
    }
    let mut out = Vec::with_capacity(count);
    for chunk in data.chunks_exact(4).take(count) {
        out.push(f32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn frame_byte_len(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
}
