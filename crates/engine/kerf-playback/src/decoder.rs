//! The picture of a clip of media, decoded in this process for playback that needs no
//! compositor: at each frame of the program, the frame of the media the clip's time map names
//! (`kerf_model::TimeMap::frame`), which is the one the compositor draws there.

use crate::libav;
use crate::timeline::{PlaybackClip, PlaybackSource, PlaybackTimeline};
use crate::PlaybackError;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::context::Input;
use ffmpeg_next::util::format::Pixel;
use ffmpeg_next::{decoder, filter, media};
use kerf_fx::RasterStack;
use kerf_model::{Rate, Time};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A clip's masks and raster effects, run on each picture before it is
/// letterboxed into the frame: the masks cut it as the compositor cuts a
/// layer, and the effects run as a render's bake runs them.
struct Live {
    stack: RasterStack,
    fps: f64,
}

impl Live {
    fn for_clip(clip: &PlaybackClip, fps: f64) -> Result<Option<Self>, PlaybackError> {
        let (width, height) = clip.layer;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let stack = RasterStack::new(&clip.effects, width, height, clip.seed)
            .map_err(PlaybackError::InvalidProject)?
            .with_masks(&clip.masks);
        if stack.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self { stack, fps }))
    }
}

/// A frame read from the in-process decoder before timeline metadata is
/// attached by the playback worker.
pub(crate) struct DecodedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    /// Source timestamp of the picture, when the packet carried one.
    /// Scrub previews use this so the UI can show a keyframe before the
    /// exact target has been decoded.
    pub source_time: Option<f64>,
}

/// A frame of the media as the filters leave it.
enum Picture {
    /// Letterboxed into the frame, as it is shown.
    Framed(Arc<[u8]>),
    /// At its own shape, for the clip's masks and effects to run on before it is letterboxed.
    Loose {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
}

/// One libav decoder plus filter graph. It gives a picture for each frame of the program in
/// turn from the one it opened on, reading on through the file. Small forward seeks on the
/// same clip pass frames by instead of reopening; clip changes and large or backward seeks
/// open a new context.
pub(crate) struct VideoDecoder {
    input: Input,
    decoder: decoder::Video,
    filter: Option<filter::Graph>,
    stream_index: usize,
    time_base: ffmpeg::Rational,
    /// The timestamp of the media's first frame, and the rate its frames are counted at.
    first_pts: i64,
    rate: Rate,
    source_path: PathBuf,
    clip: PlaybackClip,
    width: u32,
    height: u32,
    /// The file has been read to its end and the decoder told.
    eof: bool,
    /// The frame of the media showing, by its timestamp, and the one read after it, which
    /// shows where the first ends. A frame the filters have taken is left empty.
    current: Option<(i64, ffmpeg::frame::Video)>,
    ahead: Option<(i64, ffmpeg::frame::Video)>,
    /// The frame showing as the filters left it, by its timestamp: a frame that shows over
    /// several of the program's frames is filtered once.
    filtered: Option<(i64, Picture)>,
    /// Give the first decoded picture even when it is the keyframe before
    /// the target. Cleared after that picture is given.
    release_early: bool,
    /// The decoder was opened for the scrub preview.
    preview_graph: bool,
    cancelled: bool,
    /// The frame of the program the decoder opened on and the rate the program's frames are
    /// counted at, and how many pictures it has given since: the next is of frame
    /// `first + served`.
    first: i64,
    program_rate: Rate,
    served: u32,
    epoch: Arc<AtomicU64>,
    serve_epoch: u64,
    /// The clip's masks and raster effects, when it has any.
    live: Option<Live>,
}

impl VideoDecoder {
    #[expect(
        clippy::too_many_arguments,
        reason = "each argument is a separate part of what a clip's decoder opens on: the timeline, the clip and its source, the time, the size, whether its first picture may be early, and the epoch"
    )]
    pub fn open(
        timeline: &PlaybackTimeline,
        clip: &PlaybackClip,
        source: &PlaybackSource,
        timeline_time: f64,
        requested_width: Option<u32>,
        requested_height: Option<u32>,
        release_early: bool,
        epoch: Arc<AtomicU64>,
        serve_epoch: u64,
    ) -> Result<Self, PlaybackError> {
        libav::ensure_init()?;
        let (width, height) = frame_size(timeline, requested_width, requested_height);

        let input = ffmpeg::format::input(&source.path).map_err(libav::map_error)?;
        let (stream_index, time_base, first_pts, parameters) = {
            let stream = input
                .streams()
                .best(media::Type::Video)
                .ok_or_else(|| PlaybackError::Ffmpeg("no video stream".into()))?;
            let start = stream.start_time();
            (
                stream.index(),
                stream.time_base(),
                if start == ffmpeg::ffi::AV_NOPTS_VALUE {
                    0
                } else {
                    start
                },
                stream.parameters(),
            )
        };
        let decoder = ffmpeg::codec::context::Context::from_parameters(parameters)
            .map_err(libav::map_error)?
            .decoder()
            .video()
            .map_err(libav::map_error)?;

        let mut decoder = Self {
            input,
            decoder,
            filter: None,
            stream_index,
            time_base,
            first_pts,
            rate: source.rate,
            source_path: source.path.clone(),
            clip: clip.clone(),
            width,
            height,
            eof: false,
            current: None,
            ahead: None,
            filtered: None,
            release_early,
            preview_graph: release_early,
            cancelled: false,
            first: 0,
            program_rate: timeline.rate(),
            served: 0,
            epoch,
            serve_epoch,
            live: Live::for_clip(clip, timeline.fps)?,
        };
        decoder.aim(timeline, timeline_time);
        let graph = match decoder.build_filter(timeline, source) {
            Ok(graph) => graph,
            Err(error) => {
                if timeline.subtitles.is_empty() && source.subtitles.is_empty() {
                    return Err(error);
                }
                decoder.build_filter_without_subtitles(timeline, source)?
            }
        };
        decoder.filter = Some(graph);
        Ok(decoder)
    }

    pub fn clip_id(&self) -> &str {
        &self.clip.id
    }

    pub fn same_source(&self, path: &Path) -> bool {
        self.source_path == path
    }

    pub fn is_preview_graph(&self) -> bool {
        self.preview_graph
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Go on decoding for a newer seek that stays where this decoder is.
    pub fn serve(&mut self, serve_epoch: u64) {
        self.serve_epoch = serve_epoch;
    }

    /// Seek inside the open file and rebuild the filter for a new target.
    /// Reopening the container on every pointer move is what makes a scrub stall.
    pub fn retarget(
        &mut self,
        timeline: &PlaybackTimeline,
        clip: &PlaybackClip,
        source: &PlaybackSource,
        timeline_time: f64,
        release_early: bool,
        serve_epoch: u64,
    ) -> Result<(), PlaybackError> {
        if source.path != self.source_path {
            return Err(PlaybackError::Ffmpeg(
                "cannot retarget a decoder onto a different file".into(),
            ));
        }
        self.clip = clip.clone();
        self.rate = source.rate;
        self.program_rate = timeline.rate();
        self.cancelled = false;
        self.release_early = release_early;
        self.preview_graph = release_early;
        self.serve_epoch = serve_epoch;
        self.live = Live::for_clip(clip, timeline.fps)?;
        self.aim(timeline, timeline_time);
        self.filter = None;
        let graph = match self.build_filter(timeline, source) {
            Ok(graph) => graph,
            Err(error) => {
                if release_early || (timeline.subtitles.is_empty() && source.subtitles.is_empty()) {
                    return Err(error);
                }
                self.build_filter_without_subtitles(timeline, source)?
            }
        };
        self.filter = Some(graph);
        Ok(())
    }

    /// Start over at the frame of the program showing at `timeline_time`: forget what was
    /// read, and seek the file to the frame of the media the clip shows there.
    fn aim(&mut self, timeline: &PlaybackTimeline, timeline_time: f64) {
        self.first = timeline.frame_at(timeline_time);
        self.served = 0;
        self.current = None;
        self.ahead = None;
        self.filtered = None;
        self.eof = false;
        let (num, den) = self.time_base_parts();
        let pts = i128::from(self.pts_of(self.wanted(0)));
        let micros = (pts * i128::from(num) * 1_000_000).div_euclid(i128::from(den)) as i64;
        self.decoder.flush();
        libav::seek_to(&mut self.input, micros);
        self.decoder.flush();
    }

    /// The frame of the media the clip shows at the `ahead`th frame of the program from the
    /// one the decoder opened on.
    fn wanted(&self, ahead: u32) -> i64 {
        let at = self.program_rate.time_of(self.first + i64::from(ahead));
        self.clip.source_frame_at(at, self.rate)
    }

    fn time_base_parts(&self) -> (i64, i64) {
        (
            i64::from(self.time_base.numerator()),
            i64::from(self.time_base.denominator()).max(1),
        )
    }

    /// Where frame `index` of the media starts, in the stream's time base, as the compositor
    /// finds it.
    fn pts_of(&self, index: i64) -> i64 {
        let (num, den) = self.time_base_parts();
        let ticks = i128::from(index) * i128::from(self.rate.den()) * i128::from(den);
        let per = i128::from(self.rate.num()) * i128::from(num.max(1));
        self.first_pts + (ticks + per / 2).div_euclid(per) as i64
    }

    /// The picture of the next frame of the program, or nothing when the decoder was
    /// cancelled or the file gave no frame.
    pub fn next_frame(&mut self) -> Result<Option<DecodedVideoFrame>, PlaybackError> {
        if self.note_cancel() {
            return Ok(None);
        }
        if self.release_early {
            // The first picture after the seek, usually the keyframe before the frame asked
            // for, is shown as it is, where the clip plays it.
            let Some((pts, mut frame)) = self.read()? else {
                return Ok(None);
            };
            self.release_early = false;
            let picture = self.filter_frame(&mut frame)?;
            let source_time = libav::seconds(pts, self.time_base);
            let at = self.clip.clip_time_of_source(source_time);
            return Ok(Some(self.give(&picture, at, source_time)));
        }
        let index = self.wanted(self.served);
        self.read_to(index)?;
        if self.cancelled {
            return Ok(None);
        }
        // Before the media's first frame, its first frame shows.
        if self.current.is_none() {
            self.current = self.ahead.take();
        }
        let Some((pts, frame)) = self.current.as_mut() else {
            return Ok(None);
        };
        let pts = *pts;
        if self.filtered.as_ref().is_none_or(|(of, _)| *of != pts) {
            let mut frame = std::mem::replace(frame, ffmpeg::frame::Video::empty());
            let picture = self.filter_frame(&mut frame)?;
            self.filtered = Some((pts, picture));
        }
        let at = self.clip.clip_time_at(
            self.program_rate
                .time_of(self.first + i64::from(self.served)),
        );
        self.served += 1;
        let (_, picture) = self
            .filtered
            .take()
            .expect("the frame showing was filtered just above, or on an earlier call");
        let given = self.give(&picture, at, libav::seconds(pts, self.time_base));
        self.filtered = Some((pts, picture));
        Ok(Some(given))
    }

    /// Pass the next frame of the program by without making its picture. A seek a little
    /// way on does this for the frames before its target; the frames of the media they would
    /// have shown are read past when the next picture is asked for.
    pub fn skip_frame(&mut self) -> Result<bool, PlaybackError> {
        if self.note_cancel() {
            return Ok(false);
        }
        self.served += 1;
        Ok(true)
    }

    pub fn stop(&mut self) {
        self.eof = true;
        self.current = None;
        self.ahead = None;
        self.filtered = None;
        self.filter = None;
    }

    /// Read on until the frame showing is frame `index` of the media: the last frame
    /// starting by its start, give or take half a frame for timestamps that round, as the
    /// compositor takes it. A file that ends before then shows its last frame.
    fn read_to(&mut self, index: i64) -> Result<(), PlaybackError> {
        let limit = (self.pts_of(index) + self.pts_of(index + 1)) / 2;
        loop {
            if self.ahead.is_none() {
                self.ahead = self.read()?;
            }
            match &self.ahead {
                Some((pts, _)) if *pts <= limit => self.current = self.ahead.take(),
                _ => return Ok(()),
            }
        }
    }

    /// The next frame of the file in the order it shows, with its timestamp; nothing at the
    /// file's end, or once the decoder is cancelled.
    fn read(&mut self) -> Result<Option<(i64, ffmpeg::frame::Video)>, PlaybackError> {
        loop {
            if self.note_cancel() {
                return Ok(None);
            }
            let mut decoded = ffmpeg::frame::Video::empty();
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    // A frame with no timestamp follows the one before it.
                    let pts = decoded.timestamp().unwrap_or_else(|| {
                        self.ahead
                            .as_ref()
                            .or(self.current.as_ref())
                            .map_or(self.first_pts, |(pts, _)| {
                                pts + self.pts_of(1) - self.first_pts
                            })
                    });
                    return Ok(Some((pts, decoded)));
                }
                Err(error) if libav::is_eof(&error) => return Ok(None),
                Err(error) if libav::is_again(&error) => {}
                Err(error) => return Err(libav::map_error(error)),
            }
            if self.eof {
                return Ok(None);
            }
            let packet = self
                .input
                .packets()
                .next()
                .map(|(stream, packet)| (stream.index(), packet));
            match packet {
                Some((index, packet)) if index == self.stream_index => {
                    match self.decoder.send_packet(&packet) {
                        Ok(()) => {}
                        Err(error) if libav::is_again(&error) => {}
                        Err(error) if libav::is_eof(&error) => self.eof = true,
                        Err(error) => return Err(libav::map_error(error)),
                    }
                }
                Some(_) => {}
                None => {
                    self.eof = true;
                    if let Err(error) = self.decoder.send_eof() {
                        log::warn!(
                            "the decoder was not told its input ended, so it may hold back its last frames: {error}"
                        );
                    }
                }
            }
        }
    }

    /// `frame` through the filters, which take it and leave it empty: scaled to the frame,
    /// and letterboxed into it unless the clip's masks and effects are still to run.
    fn filter_frame(&mut self, frame: &mut ffmpeg::frame::Video) -> Result<Picture, PlaybackError> {
        let filter = self
            .filter
            .as_mut()
            .ok_or_else(|| PlaybackError::Ffmpeg("video filter graph is missing".into()))?;
        filter
            .get("in")
            .ok_or_else(|| PlaybackError::Ffmpeg("missing buffer source".into()))?
            .source()
            .add(frame)
            .map_err(libav::map_error)?;
        let mut filtered = ffmpeg::frame::Video::empty();
        filter
            .get("out")
            .ok_or_else(|| PlaybackError::Ffmpeg("missing buffersink".into()))?
            .sink()
            .frame(&mut filtered)
            .map_err(libav::map_error)?;
        if self.live.is_none() {
            return Ok(Picture::Framed(libav::packed_rgba(
                &filtered,
                self.width,
                self.height,
            )?));
        }
        let (width, height) = (filtered.width(), filtered.height());
        Ok(Picture::Loose {
            rgba: libav::packed_rgba(&filtered, width, height)?.to_vec(),
            width,
            height,
        })
    }

    /// `picture` as it is shown at the clip's own time `at`: with the clip's masks and
    /// effects run on it as they are then, and letterboxed into the frame.
    fn give(&self, picture: &Picture, at: Time, source_time: f64) -> DecodedVideoFrame {
        let pixels = match (picture, &self.live) {
            (Picture::Framed(pixels), _) => Arc::clone(pixels),
            (
                Picture::Loose {
                    rgba,
                    width,
                    height,
                },
                live,
            ) => {
                let mut rgba = rgba.clone();
                if let Some(live) = live {
                    live.stack.apply(&mut rgba, *width, *height, at, live.fps);
                }
                letterbox(&rgba, *width, *height, self.width, self.height).into()
            }
        };
        DecodedVideoFrame {
            width: self.width,
            height: self.height,
            pixels,
            source_time: Some(source_time),
        }
    }

    fn build_filter(
        &self,
        timeline: &PlaybackTimeline,
        source: &PlaybackSource,
    ) -> Result<filter::Graph, PlaybackError> {
        self.build_filter_graph(timeline, source, true)
    }

    fn build_filter_without_subtitles(
        &self,
        timeline: &PlaybackTimeline,
        source: &PlaybackSource,
    ) -> Result<filter::Graph, PlaybackError> {
        self.build_filter_graph(timeline, source, false)
    }

    fn build_filter_graph(
        &self,
        timeline: &PlaybackTimeline,
        source: &PlaybackSource,
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
        // A clip with masks or raster effects leaves the graph
        // unletterboxed, and without burned-in subtitles, for them to run
        // first. The scrub preview burns none in either.
        let letterbox = self.live.is_none();
        let spec = video_filter(
            timeline,
            self.width,
            self.height,
            include_subtitles && letterbox && !self.release_early,
            letterbox,
            source,
        );
        let mut graph = filter::Graph::new();
        let buffer = filter::find("buffer").ok_or_else(|| {
            PlaybackError::Ffmpeg("libavfilter buffer source is unavailable".into())
        })?;
        let sink = filter::find("buffersink")
            .ok_or_else(|| PlaybackError::Ffmpeg("libavfilter buffersink is unavailable".into()))?;
        graph.add(&buffer, "in", &args).map_err(libav::map_error)?;
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

    fn note_cancel(&mut self) -> bool {
        if self.cancelled {
            return true;
        }
        if self.epoch.load(Ordering::Acquire) != self.serve_epoch {
            self.cancelled = true;
            return true;
        }
        false
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The filters between a frame as it decoded and the picture shown: it is fitted to the frame,
/// letterboxed into it, and its subtitles burned in. Which frame shows when is not theirs to
/// say: the decoder hands them the frame the clip's time map names.
fn video_filter(
    timeline: &PlaybackTimeline,
    width: u32,
    height: u32,
    include_subtitles: bool,
    letterbox: bool,
    source: &PlaybackSource,
) -> String {
    let mut filters = vec![format!(
        "scale=w={width}:h={height}:force_original_aspect_ratio=decrease"
    )];
    if letterbox {
        filters.push(format!(
            "pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
        ));
    }
    filters.push("setsar=1".to_string());
    if include_subtitles {
        for subtitle in &source.subtitles {
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

/// The size the program's frames are given at: the size asked for, no larger than the
/// program's, and 2 by 2 at least.
pub(crate) fn frame_size(
    timeline: &PlaybackTimeline,
    requested_width: Option<u32>,
    requested_height: Option<u32>,
) -> (u32, u32) {
    let width = requested_width.map_or(timeline.width, |width| width.min(timeline.width));
    let height = requested_height.map_or(timeline.height, |height| height.min(timeline.height));
    (width.max(2), height.max(2))
}

/// An opaque black RGBA picture: the program's empty canvas, as the compositor draws it.
pub(crate) fn black(width: u32, height: u32) -> Vec<u8> {
    let mut frame = vec![0; width as usize * height as usize * 4];
    for alpha in frame.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    frame
}

/// Center an RGBA picture on a black frame, as the graph's `pad` would,
/// showing black through whatever the masks or effects made transparent.
fn letterbox(
    picture: &[u8],
    width: u32,
    height: u32,
    frame_width: u32,
    frame_height: u32,
) -> Vec<u8> {
    let mut frame = black(frame_width, frame_height);
    let (left, top) = (
        (frame_width.saturating_sub(width) / 2) as usize,
        (frame_height.saturating_sub(height) / 2) as usize,
    );
    let columns = width.min(frame_width) as usize;
    for row in 0..height.min(frame_height) as usize {
        for column in 0..columns {
            let from = (row * width as usize + column) * 4;
            let to = ((row + top) * frame_width as usize + column + left) * 4;
            let alpha = u16::from(picture[from + 3]);
            for channel in 0..3 {
                frame[to + channel] =
                    ((u16::from(picture[from + channel]) * alpha + 127) / 255) as u8;
            }
        }
    }
    frame
}

fn pixel_name(format: Pixel) -> String {
    format
        .descriptor()
        .map(|descriptor| descriptor.name().to_string())
        .filter(|name| !name.is_empty() && name != "none")
        .unwrap_or_else(|| "yuv420p".into())
}
