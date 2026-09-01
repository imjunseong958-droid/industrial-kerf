use crate::timeline::{PlaybackTimeline, ResolvedClip};
use crate::PlaybackError;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Arc;

/// A frame read from the FFmpeg rawvideo pipe before timeline metadata is
/// attached by the playback worker.
#[derive(Debug)]
pub(crate) struct DecodedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

/// One short-lived FFmpeg decoder. A decoder is restarted at clip changes and
/// seeks, which makes long-GOP seeking deterministic without blocking the UI.
#[derive(Debug)]
pub(crate) struct VideoDecoder {
    child: Child,
    stdout: ChildStdout,
    clip_id: String,
    width: u32,
    height: u32,
}

impl VideoDecoder {
    pub fn open(
        ffmpeg_binary: &Path,
        timeline: &PlaybackTimeline,
        clip: &ResolvedClip,
        timeline_time: f64,
        requested_width: Option<u32>,
        requested_height: Option<u32>,
    ) -> Result<Self, PlaybackError> {
        let source_time = clip.source_time_at(timeline_time);
        let remaining_source = (clip.clip.source_out - source_time).max(0.0);
        if remaining_source <= 0.0 {
            return Err(PlaybackError::Ffmpeg(format!(
                "clip {} has no source time remaining",
                clip.clip.id
            )));
        }
        let width = requested_width.unwrap_or(timeline.width).max(2);
        let height = requested_height.unwrap_or(timeline.height).max(2);
        let filter = video_filter(timeline, clip, width, height, remaining_source);
        let binary = ffmpeg_binary.to_string_lossy();
        let mut args = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-nostdin".into(),
            // Put the seek before the input. FFmpeg seeks to the preceding
            // keyframe, decodes forward accurately, and then feeds the
            // requested timeline into the filter graph. An output-side
            // `-ss` would run after `setpts=PTS-STARTPTS` and discard the
            // entire filtered stream as an output timestamp seek.
            "-ss".into(),
            format_number(source_time),
            "-i".into(),
            clip.source.path.to_string_lossy().into_owned(),
            "-an".into(),
            "-vf".into(),
            filter,
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "pipe:1".into(),
        ];
        if let Some(value) = std::env::var_os("CUT_HWACCEL") {
            let value = value.to_string_lossy();
            if !value.trim().is_empty() && !value.eq_ignore_ascii_case("none") {
                args.splice(0..0, ["-hwaccel".into(), value.to_ascii_lowercase()]);
            }
        }

        let mut command = Command::new(binary.as_ref());
        let child = command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                PlaybackError::Ffmpeg(format!("failed to start {}: {}", binary, error))
            })?;
        let mut child = child;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PlaybackError::Ffmpeg("ffmpeg decoder did not expose stdout".into()))?;
        Ok(Self {
            child,
            stdout,
            clip_id: clip.clip.id.clone(),
            width,
            height,
        })
    }

    pub fn clip_id(&self) -> &str {
        &self.clip_id
    }

    pub fn next_frame(&mut self) -> Result<Option<DecodedVideoFrame>, PlaybackError> {
        let byte_len = frame_byte_len(self.width, self.height).ok_or_else(|| {
            PlaybackError::Ffmpeg(format!(
                "decoded frame dimensions are too large: {}x{}",
                self.width, self.height
            ))
        })?;
        let mut pixels = vec![0_u8; byte_len];
        match self.stdout.read_exact(&mut pixels) {
            Ok(()) => Ok(Some(DecodedVideoFrame {
                width: self.width,
                height: self.height,
                pixels: Arc::from(pixels.into_boxed_slice()),
            })),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(PlaybackError::Ffmpeg(format!(
                "reading decoded frame failed: {error}"
            ))),
        }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn video_filter(
    timeline: &PlaybackTimeline,
    clip: &ResolvedClip,
    width: u32,
    height: u32,
    remaining_source: f64,
) -> String {
    let speed = format_number(clip.clip.speed);
    let fps = format_number(timeline.fps);
    let mut filters = vec![
        "setpts=PTS-STARTPTS".to_string(),
        // `-ss` is intentionally after `-i` for accurate long-GOP seeks. In
        // that mode FFmpeg can retain the source timestamp on decoded
        // frames, so normalize PTS before applying the remaining-duration
        // trim or the filter would discard every frame after a seek.
        format!("trim=duration={}", format_number(remaining_source)),
        format!("setpts=PTS/{speed}"),
        format!("fps={fps}"),
        format!("scale=w={width}:h={height}:force_original_aspect_ratio=decrease"),
        format!("pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"),
        "setsar=1".to_string(),
    ];
    for subtitle in &clip.source.subtitles {
        if subtitle.is_file() {
            filters.push(format!(
                "subtitles=filename='{}'",
                escape_filter_path(subtitle)
            ));
        }
    }
    for subtitle in &timeline.subtitles {
        if subtitle.is_file() {
            filters.push(format!(
                "subtitles=filename='{}'",
                escape_filter_path(subtitle)
            ));
        }
    }
    filters.push("format=rgba".to_string());
    filters.join(",")
}

fn escape_filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
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

fn format_number(value: f64) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    format!("{value:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
