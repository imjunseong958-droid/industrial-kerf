use crate::libav;
use crate::timeline::{PlaybackTimeline, ResolvedClip};
use crate::PlaybackError;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::context::Input;
use ffmpeg_next::util::format::Pixel;
use ffmpeg_next::{decoder, filter, media};
use std::path::Path;
use std::sync::Arc;

/// A frame read from the in-process decoder before timeline metadata is
/// attached by the playback worker.
pub(crate) struct DecodedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

/// One libav decoder plus filter graph. Small forward seeks on the same clip
/// drop frames instead of reopening; clip changes and large/backward seeks
/// open a new context.
pub(crate) struct VideoDecoder {
    input: Input,
    decoder: decoder::Video,
    filter: Option<filter::Graph>,
    stream_index: usize,
    time_base: ffmpeg::Rational,
    source_time: f64,
    clip_id: String,
    width: u32,
    height: u32,
    eof: bool,
    flushed: bool,
}

impl VideoDecoder {
    pub fn open(
        _ffmpeg_binary: &Path,
        timeline: &PlaybackTimeline,
        clip: &ResolvedClip,
        timeline_time: f64,
        requested_width: Option<u32>,
        requested_height: Option<u32>,
    ) -> Result<Self, PlaybackError> {
        libav::ensure_init()?;
        let source_time = clip.source_time_at(timeline_time);
        let remaining_source = (clip.clip.source_out - source_time).max(0.0);
        if remaining_source <= 0.0 {
            return Err(PlaybackError::Ffmpeg(format!(
                "clip {} has no source time remaining",
                clip.clip.id
            )));
        }
        let width = match requested_width {
            Some(width) => width.min(timeline.width).max(2),
            None => timeline.width.max(2),
        };
        let height = match requested_height {
            Some(height) => height.min(timeline.height).max(2),
            None => timeline.height.max(2),
        };

        let mut input = ffmpeg::format::input(&clip.source.path).map_err(libav::map_error)?;
        let (stream_index, time_base) = {
            let stream = input
                .streams()
                .best(media::Type::Video)
                .ok_or_else(|| PlaybackError::Ffmpeg("no video stream".into()))?;
            (stream.index(), stream.time_base())
        };
        let decoder = {
            let stream = input
                .streams()
                .best(media::Type::Video)
                .ok_or_else(|| PlaybackError::Ffmpeg("no video stream".into()))?;
            let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
                .map_err(libav::map_error)?;
            context.decoder().video().map_err(libav::map_error)?
        };
        libav::seek_to(&mut input, source_time);

        let mut decoder = Self {
            input,
            decoder,
            filter: None,
            stream_index,
            time_base,
            source_time,
            clip_id: clip.clip.id.clone(),
            width,
            height,
            eof: false,
            flushed: false,
        };
        let graph = match decoder.build_filter(timeline, clip, remaining_source) {
            Ok(graph) => graph,
            Err(error) => {
                if timeline.subtitles.is_empty() && clip.source.subtitles.is_empty() {
                    return Err(error);
                }
                decoder.build_filter_without_subtitles(timeline, clip, remaining_source)?
            }
        };
        decoder.filter = Some(graph);
        Ok(decoder)
    }

    pub fn clip_id(&self) -> &str {
        &self.clip_id
    }

    pub fn next_frame(&mut self) -> Result<Option<DecodedVideoFrame>, PlaybackError> {
        loop {
            if let Some(frame) = self.pull_filtered()? {
                return Ok(Some(frame));
            }
            if self.eof && self.flushed {
                return Ok(None);
            }
            self.pump()?;
        }
    }

    pub fn stop(&mut self) {
        self.eof = true;
        self.flushed = true;
        self.filter = None;
    }

    fn build_filter(
        &self,
        timeline: &PlaybackTimeline,
        clip: &ResolvedClip,
        remaining_source: f64,
    ) -> Result<filter::Graph, PlaybackError> {
        self.build_filter_graph(timeline, clip, remaining_source, true)
    }

    fn build_filter_without_subtitles(
        &self,
        timeline: &PlaybackTimeline,
        clip: &ResolvedClip,
        remaining_source: f64,
    ) -> Result<filter::Graph, PlaybackError> {
        self.build_filter_graph(timeline, clip, remaining_source, false)
    }

    fn build_filter_graph(
        &self,
        timeline: &PlaybackTimeline,
        clip: &ResolvedClip,
        remaining_source: f64,
        include_subtitles: bool,
    ) -> Result<filter::Graph, PlaybackError> {
        let pix_fmt = pixel_name(self.decoder.format());
        let width = self.decoder.width().max(2);
        let height = self.decoder.height().max(2);
        let time_base = self.time_base;
        let sar = self.decoder.aspect_ratio();
        let sar = if sar.numerator() == 0 {
            ffmpeg::Rational(1, 1)
        } else {
            sar
        };
        let args = format!(
            "video_size={width}x{height}:pix_fmt={pix_fmt}:time_base={}/{}:pixel_aspect={}/{}",
            time_base.numerator(),
            time_base.denominator().max(1),
            sar.numerator(),
            sar.denominator().max(1)
        );
        let spec = video_filter(
            timeline,
            clip,
            self.width,
            self.height,
            remaining_source,
            include_subtitles,
        );
        let mut graph = filter::Graph::new();
        let buffer = filter::find("buffer").ok_or_else(|| {
            PlaybackError::Ffmpeg("libavfilter buffer source is unavailable".into())
        })?;
        let sink = filter::find("buffersink").ok_or_else(|| {
            PlaybackError::Ffmpeg("libavfilter buffersink is unavailable".into())
        })?;
        graph
            .add(&buffer, "in", &args)
            .map_err(libav::map_error)?;
        graph.add(&sink, "out", "").map_err(libav::map_error)?;
        graph
            .output("in", 0)
            .map_err(libav::map_error)?
            .input("out", 0)
            .map_err(libav::map_error)?
            .parse(&spec)
            .map_err(libav::map_error)?;
        graph.validate().map_err(libav::map_error)?;
        Ok(graph)
    }

    fn pull_filtered(&mut self) -> Result<Option<DecodedVideoFrame>, PlaybackError> {
        if let Some(filter) = self.filter.as_mut() {
            let mut filtered = ffmpeg::frame::Video::empty();
            let result = {
                let mut sink = filter
                    .get("out")
                    .ok_or_else(|| PlaybackError::Ffmpeg("missing buffersink".into()))?;
                sink.sink().frame(&mut filtered)
            };
            return match result {
                Ok(()) => Ok(Some(DecodedVideoFrame {
                    width: self.width,
                    height: self.height,
                    pixels: libav::packed_rgba(&filtered, self.width, self.height)?,
                })),
                Err(error) if libav::is_again(&error) || libav::is_eof(&error) => Ok(None),
                Err(error) => Err(libav::map_error(error)),
            };
        }
        Ok(None)
    }

    fn pump(&mut self) -> Result<(), PlaybackError> {
        if !self.eof {
            let packet = self
                .input
                .packets()
                .next()
                .map(|(stream, packet)| (stream.index(), packet));
            match packet {
                Some((index, packet)) if index == self.stream_index => {
                    self.send_packet(&packet)?;
                    self.drain_decoder()?;
                    return Ok(());
                }
                Some(_) => return Ok(()),
                None => {
                    self.eof = true;
                    let _ = self.decoder.send_eof();
                    self.drain_decoder()?;
                    return Ok(());
                }
            }
        }
        if !self.flushed {
            self.flushed = true;
            if let Some(filter) = self.filter.as_mut() {
                if let Some(mut src) = filter.get("in") {
                    let _ = src.source().flush();
                }
            }
        }
        Ok(())
    }

    fn send_packet(&mut self, packet: &ffmpeg::Packet) -> Result<(), PlaybackError> {
        loop {
            match self.decoder.send_packet(packet) {
                Ok(()) => return Ok(()),
                Err(error) if libav::is_again(&error) => self.drain_decoder()?,
                Err(error) if libav::is_eof(&error) => {
                    self.eof = true;
                    return Ok(());
                }
                Err(error) => return Err(libav::map_error(error)),
            }
        }
    }

    fn drain_decoder(&mut self) -> Result<(), PlaybackError> {
        let mut decoded = ffmpeg::frame::Video::empty();
        loop {
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => self.push_decoded(&decoded)?,
                Err(error) if libav::is_again(&error) || libav::is_eof(&error) => return Ok(()),
                Err(error) => return Err(libav::map_error(error)),
            }
        }
    }

    fn push_decoded(&mut self, decoded: &ffmpeg::frame::Video) -> Result<(), PlaybackError> {
        if let Some(ts) = decoded.timestamp() {
            if libav::seconds(ts, self.time_base) + 1e-4 < self.source_time {
                return Ok(());
            }
        }
        let filter = self
            .filter
            .as_mut()
            .ok_or_else(|| PlaybackError::Ffmpeg("video filter graph is missing".into()))?;
        let mut src = filter
            .get("in")
            .ok_or_else(|| PlaybackError::Ffmpeg("missing buffer source".into()))?;
        src.source().add(decoded).map_err(libav::map_error)
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
    include_subtitles: bool,
) -> String {
    let speed = libav::format_number(clip.clip.speed);
    let fps = libav::format_number(timeline.fps);
    let mut filters = vec![
        "setpts=PTS-STARTPTS".to_string(),
        format!("trim=duration={}", libav::format_number(remaining_source)),
        format!("setpts=PTS/{speed}"),
        format!("fps={fps}"),
        format!("scale=w={width}:h={height}:force_original_aspect_ratio=decrease"),
        format!("pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"),
        "setsar=1".to_string(),
    ];
    if include_subtitles {
        for subtitle in &clip.source.subtitles {
            if subtitle.is_file() {
                filters.push(format!(
                    "subtitles=filename='{}'",
                    libav::escape_filter_path(subtitle)
                ));
            }
        }
        for subtitle in &timeline.subtitles {
            if subtitle.is_file() {
                filters.push(format!(
                    "subtitles=filename='{}'",
                    libav::escape_filter_path(subtitle)
                ));
            }
        }
    }
    filters.push("format=rgba".to_string());
    filters.join(",")
}

fn pixel_name(format: Pixel) -> String {
    format
        .descriptor()
        .map(|descriptor| descriptor.name().to_string())
        .filter(|name| !name.is_empty() && name != "none")
        .unwrap_or_else(|| "yuv420p".into())
}
