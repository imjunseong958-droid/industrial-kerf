//! Footage for the renderer: the frames each media clip shows, decoded by
//! libav.
//!
//! Every media clip is a footage item of its own, and its layer's time is
//! clip time, so the frame a clip shows is worked out here, exactly, from
//! the clip's source in and speed. A frame is the one whose display starts
//! at or before that source time, as a cut shows it. Decoders keep their
//! place in a file and read on to the next frame; they seek only to go
//! back or far ahead. A frame just after one being read waits for that
//! decoder rather than opening another, so frames drawn on several threads
//! still decode each file once. Frames are converted to RGBA by
//! libavfilter, as the graph converts them, and kept in a bounded cache.
//! A clip never shown at its full size is converted at the size it is
//! shown, as the graph scales it. [`Pictures`] gives them to the compositor
//! in linear light.

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::context::Input;
use ffmpeg_next::{decoder, filter, frame, media};
use kerf_color::{ColorPipeline, ColorSpace, InputTransform, Range};
use kerf_draw::Image as Picture;
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Rate, Ratio, Time};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// Decoders kept open across reads.
const MAX_DECODERS: usize = 8;

/// How far ahead a decoder reads on instead of seeking.
const READ_AHEAD_SECONDS: i64 = 2;

/// Decoded frames kept by default.
const CACHE_BUDGET: usize = 512 << 20;

/// How footage in `space` becomes working light. Made once each: each holds tables.
pub(crate) fn input_transform(space: ColorSpace, hdr: bool) -> &'static InputTransform {
    static MADE: Mutex<BTreeMap<(&'static str, bool), &'static InputTransform>> =
        Mutex::new(BTreeMap::new());
    lock(&MADE).entry((space.id(), hdr)).or_insert_with(|| {
        // Cameras record in video range, and the file's own range is taken out in decoding.
        Box::leak(Box::new(if hdr {
            InputTransform::extended(space, Range::Limited)
        } else {
            InputTransform::new(space, Range::Limited, &ColorPipeline::REC709, None)
        }))
    })
}

/// What a media clip shows: its media's frames, from `source_in` at
/// `speed`.
#[derive(Clone, Debug)]
pub(crate) struct ClipSource {
    pub path: String,
    /// The media's picture rate and length in frames.
    pub rate: Rate,
    pub frames: i64,
    pub width: u32,
    pub height: u32,
    /// The picture's transparency counts; otherwise it is opaque.
    pub alpha: bool,
    /// How its 16-bit code values become light, for footage worth more than 8 bits.
    pub deep: Option<&'static InputTransform>,
    pub source_in: Time,
    pub speed: Ratio,
    pub duration: Time,
}

impl ClipSource {
    /// The media frame shown at clip time `at`: the one showing at its
    /// source time, which a reversed clip runs back from its source out.
    pub fn frame_at_time(&self, at: Time) -> i64 {
        let at = i128::from(at.flicks());
        let per_second = i128::from(FLICKS_PER_SECOND);
        let units = |time: Time| i128::from(time.flicks());
        let (num, den) = (
            i128::from(self.speed.num().unsigned_abs()),
            i128::from(self.speed.den()),
        );
        // Frames per unit, as a fraction over the clip speed's denominator.
        let per = per_second * i128::from(self.rate.den()) * den;
        let frame = if self.speed.is_reverse() {
            // The frame showing just before the source position.
            let out = units(self.source_in + self.speed.source_span(self.duration));
            let position = (out * den - at * num) * i128::from(self.rate.num());
            -(-position).div_euclid(per) - 1
        } else {
            let position = (units(self.source_in) * den + at * num) * i128::from(self.rate.num());
            position.div_euclid(per)
        };
        frame.clamp(0, i128::from(self.frames.max(1) - 1)) as i64
    }
}

/// The frames in a media of `duration` at `rate`, counting a last partial
/// frame.
pub(crate) fn frame_count(rate: Rate, duration: Time) -> i64 {
    let per = i128::from(FLICKS_PER_SECOND) * i128::from(rate.den());
    let span = i128::from(duration.flicks()) * i128::from(rate.num());
    (-(-span).div_euclid(per)) as i64
}

/// Open decoders, shared by reads on several threads, and the files that
/// could not be read, with why.
pub(crate) struct DecoderPool {
    decoders: Mutex<Decoders>,
    /// Signalled when a decoder comes back.
    returned: Condvar,
    problems: Mutex<BTreeMap<String, String>>,
}

/// Open decoders: those free, and those being read with the frame each
/// reads to.
#[derive(Default)]
struct Decoders {
    idle: Vec<Decoder>,
    busy: Vec<(u64, String, i64)>,
    lent: u64,
}

impl Decoders {
    fn lend(&mut self, path: &str, index: i64) -> u64 {
        self.lent += 1;
        self.busy.push((self.lent, path.to_owned(), index));
        self.lent
    }
}

impl DecoderPool {
    fn new() -> Self {
        Self {
            decoders: Mutex::new(Decoders::default()),
            returned: Condvar::new(),
            problems: Mutex::new(BTreeMap::new()),
        }
    }

    /// Files that could not be read, with why.
    pub fn problems(&self) -> Vec<(String, String)> {
        lock(&self.problems)
            .iter()
            .map(|(path, problem)| (path.clone(), problem.clone()))
            .collect()
    }

    /// Frame `index` of `clip`'s file as straight RGBA at `size`, 16 bits a component for deep
    /// footage; none for a file that could not be read, whose problem is kept.
    fn picture(&self, clip: &ClipSource, index: i64, size: (u32, u32)) -> Option<frame::Video> {
        let (loan, mut decoder) = match self.checkout(clip, index) {
            Ok(checked_out) => checked_out,
            Err(problem) => {
                lock(&self.problems).insert(clip.path.clone(), problem);
                return None;
            }
        };
        let read = decoder.picture(index, size, clip.deep.is_some());
        self.give_back(loan, decoder);
        read.inspect_err(|problem| {
            lock(&self.problems).insert(clip.path.clone(), problem.clone());
        })
        .ok()
    }

    /// A decoder for `clip`'s file, lent with a token to give it back by:
    /// the free one that reads on to `index` soonest; else, after it, one
    /// being read to just before `index`; else one that seeks, or a new
    /// one.
    fn checkout(&self, clip: &ClipSource, index: i64) -> Result<(u64, Decoder), String> {
        let ahead = i64::from(clip.rate.num()) * READ_AHEAD_SECONDS / i64::from(clip.rate.den());
        let reads_on = |at: i64| at <= index && index - at <= ahead.max(1);
        let mut decoders = lock(&self.decoders);
        loop {
            let free = decoders
                .idle
                .iter()
                .enumerate()
                .filter(|(_, decoder)| decoder.path == clip.path)
                .filter_map(|(at, decoder)| {
                    decoder
                        .position()
                        .filter(|position| reads_on(*position))
                        .map(|position| (at, position))
                })
                .max_by_key(|(_, position)| *position)
                .map(|(at, _)| at);
            if let Some(at) = free {
                let decoder = decoders.idle.remove(at);
                return Ok((decoders.lend(&clip.path, index), decoder));
            }
            let coming = decoders
                .busy
                .iter()
                .any(|(_, path, to)| *path == clip.path && reads_on(*to));
            if !coming {
                break;
            }
            decoders = self
                .returned
                .wait(decoders)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        let loan = decoders.lend(&clip.path, index);
        if let Some(at) = decoders
            .idle
            .iter()
            .rposition(|decoder| decoder.path == clip.path)
        {
            return Ok((loan, decoders.idle.remove(at)));
        }
        drop(decoders);
        Decoder::open(&clip.path, clip.rate)
            .map(|decoder| (loan, decoder))
            .inspect_err(|_| {
                lock(&self.decoders).busy.retain(|(lent, ..)| *lent != loan);
                self.returned.notify_all();
            })
    }

    fn give_back(&self, loan: u64, decoder: Decoder) {
        let mut decoders = lock(&self.decoders);
        decoders.busy.retain(|(lent, ..)| *lent != loan);
        decoders.idle.push(decoder);
        if decoders.idle.len() > MAX_DECODERS {
            decoders.idle.remove(0);
        }
        drop(decoders);
        self.returned.notify_all();
    }
}

/// The frames the compositor reads: each media clip's, in linear light, by
/// the clip's place in the scene.
pub(crate) struct Pictures {
    clips: Vec<ClipSource>,
    pool: DecoderPool,
    cache: Mutex<FrameCache<Picture>>,
    /// The frames the GPU draws from, as they decoded.
    decoded: Mutex<FrameCache<frame::Video>>,
}

impl Pictures {
    pub fn new(clips: Vec<ClipSource>) -> Self {
        Self {
            clips,
            pool: DecoderPool::new(),
            cache: Mutex::new(FrameCache::new(CACHE_BUDGET, picture_bytes)),
            decoded: Mutex::new(FrameCache::new(CACHE_BUDGET, decoded_bytes)),
        }
    }

    /// Files that could not be read, with why.
    pub fn problems(&self) -> Vec<(String, String)> {
        self.pool.problems()
    }

    /// The frame clip `index` shows at clip time `at`, decoded at `size`.
    pub fn picture(&self, index: usize, at: Time, size: (u32, u32)) -> Option<Arc<Picture>> {
        let clip = self.clips.get(index)?;
        let frame = clip.frame_at_time(at);
        let size = (size.0.max(1), size.1.max(1));
        let key = (clip.path.clone(), frame, size);
        if let Some(picture) = lock(&self.cache).get(&key) {
            return Some(picture);
        }
        let rgba = self.pool.picture(clip, frame, size)?;
        let (w, h) = (rgba.width() as usize, rgba.height() as usize);
        let picture = Arc::new(match clip.deep {
            Some(input) => {
                Picture::from_rgba16(rgba.data(0), (w, h), rgba.stride(0), clip.alpha, |code| {
                    input.convert(code)
                })
            }
            None => Picture::from_rgba8(rgba.data(0), w, h, rgba.stride(0), clip.alpha),
        });
        lock(&self.cache).insert(key, picture.clone());
        Some(picture)
    }

    /// The frame clip `index` shows at clip time `at`, decoded at `size`, as the straight RGBA it
    /// decodes to, for the GPU to take into linear light.
    pub fn decoded(&self, index: usize, at: Time, size: (u32, u32)) -> Option<Decoded> {
        let clip = self.clips.get(index)?;
        let frame = clip.frame_at_time(at);
        let size = (size.0.max(1), size.1.max(1));
        let key = (clip.path.clone(), frame, size);
        // Not held while decoding: other frames go on.
        let kept = lock(&self.decoded).get(&key);
        let picture = match kept {
            Some(picture) => picture,
            None => {
                let picture = Arc::new(self.pool.picture(clip, frame, size)?);
                lock(&self.decoded).insert(key, picture.clone());
                picture
            }
        };
        Some(Decoded {
            picture,
            alpha: clip.alpha,
            deep: clip.deep,
        })
    }

    /// The frame clip `index` shows at clip time `at`, opaque RGBA at its
    /// own size as it is decoded: for a frame that is nothing but that
    /// clip.
    pub fn rgba(&self, index: usize, at: Time) -> Option<Vec<u8>> {
        let clip = self.clips.get(index)?;
        let picture = self
            .pool
            .picture(clip, clip.frame_at_time(at), (clip.width, clip.height))?;
        Some(packed(&picture))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A frame as it decoded, and how to read it.
pub(crate) struct Decoded {
    pub picture: Arc<frame::Video>,
    pub alpha: bool,
    pub deep: Option<&'static InputTransform>,
}

type FrameKey = (String, i64, (u32, u32));

/// Decoded frames, the least recently used dropped first past the budget.
struct FrameCache<T> {
    frames: HashMap<FrameKey, (Arc<T>, u64)>,
    bytes: usize,
    budget: usize,
    clock: u64,
    weigh: fn(&T) -> usize,
}

impl<T> FrameCache<T> {
    fn new(budget: usize, weigh: fn(&T) -> usize) -> Self {
        Self {
            frames: HashMap::new(),
            bytes: 0,
            budget,
            clock: 0,
            weigh,
        }
    }

    fn get(&mut self, key: &FrameKey) -> Option<Arc<T>> {
        self.clock += 1;
        let clock = self.clock;
        self.frames.get_mut(key).map(|(image, used)| {
            *used = clock;
            image.clone()
        })
    }

    fn insert(&mut self, key: FrameKey, image: Arc<T>) {
        let bytes = (self.weigh)(&image);
        if bytes > self.budget {
            return;
        }
        self.clock += 1;
        if let Some((old, _)) = self.frames.insert(key, (image, self.clock)) {
            self.bytes -= (self.weigh)(&old);
        }
        self.bytes += bytes;
        while self.bytes > self.budget {
            let Some(oldest) = self
                .frames
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some((image, _)) = self.frames.remove(&oldest) {
                self.bytes -= (self.weigh)(&image);
            }
        }
    }
}

fn decoded_bytes(decoded: &frame::Video) -> usize {
    decoded.stride(0) * decoded.height() as usize
}

fn picture_bytes(picture: &Picture) -> usize {
    picture.px.len() * std::mem::size_of::<f32>()
}

pub(crate) fn init() -> Result<(), String> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        ffmpeg::init()
            .map(|()| ffmpeg::log::set_level(ffmpeg::log::Level::Error))
            .map_err(|error| error.to_string())
    })
    .clone()
}

/// One open file, read forward frame by frame.
struct Decoder {
    path: String,
    input: Input,
    decoder: decoder::Video,
    stream: usize,
    time_base: (i64, i64),
    /// The timestamp of the media's first frame.
    first: i64,
    rate: Rate,
    /// The last frame read and the one after it, read ahead to see where
    /// the last one ends.
    current: Option<(i64, frame::Video)>,
    ahead: Option<(i64, frame::Video)>,
    eof: bool,
    convert: Option<Convert>,
}

impl Decoder {
    fn open(path: &str, rate: Rate) -> Result<Self, String> {
        init()?;
        let failed = |error: ffmpeg::Error| format!("{path}: {error}");
        let input = ffmpeg::format::input(&path).map_err(failed)?;
        let (stream, time_base, first, parameters) = {
            let stream = input
                .streams()
                .best(media::Type::Video)
                .ok_or_else(|| format!("{path}: no picture"))?;
            let time_base = stream.time_base();
            let start = stream.start_time();
            (
                stream.index(),
                (
                    i64::from(time_base.numerator()),
                    i64::from(time_base.denominator()).max(1),
                ),
                if start == ffmpeg::ffi::AV_NOPTS_VALUE {
                    0
                } else {
                    start
                },
                stream.parameters(),
            )
        };
        let decoder = ffmpeg::codec::context::Context::from_parameters(parameters)
            .and_then(|mut context| {
                // Decode on every core, as FFmpeg's command line does.
                context.set_threading(ffmpeg::codec::threading::Config::kind(
                    ffmpeg::codec::threading::Type::Frame,
                ));
                context.decoder().video()
            })
            .map_err(failed)?;
        Ok(Self {
            path: path.to_owned(),
            input,
            decoder,
            stream,
            time_base,
            first,
            rate,
            current: None,
            ahead: None,
            eof: false,
            convert: None,
        })
    }

    /// The index of the last frame read, by the media's rate.
    fn position(&self) -> Option<i64> {
        self.current.as_ref().map(|(pts, _)| self.index_of(*pts))
    }

    /// Where frame `index` starts, in the stream's time base.
    fn pts_of(&self, index: i64) -> i64 {
        let (num, den) = self.time_base;
        let ticks = i128::from(index) * i128::from(self.rate.den()) * i128::from(den);
        let per = i128::from(self.rate.num()) * i128::from(num.max(1));
        self.first + (ticks + per / 2).div_euclid(per) as i64
    }

    fn index_of(&self, pts: i64) -> i64 {
        let (num, den) = self.time_base;
        let ticks = i128::from(pts - self.first) * i128::from(num) * i128::from(self.rate.num());
        let per = i128::from(den) * i128::from(self.rate.den());
        // Rounded, as `pts_of` rounds.
        (ticks + per / 2).div_euclid(per) as i64
    }

    /// Frame `index` as straight RGBA at `size`, 16 bits a component when `deep`: the last frame
    /// starting by its start, give or take half a frame for timestamps that round.
    fn picture(
        &mut self,
        index: i64,
        size: (u32, u32),
        deep: bool,
    ) -> Result<frame::Video, String> {
        let target = self.pts_of(index);
        let limit = (target + self.pts_of(index + 1)) / 2;
        let ahead = i64::from(self.rate.num()) * READ_AHEAD_SECONDS / i64::from(self.rate.den());
        let far = self.pts_of(index - ahead.max(1));
        let reads_on = self
            .current
            .as_ref()
            .is_some_and(|(pts, _)| (far..=limit).contains(pts));
        if !reads_on {
            self.seek(target)?;
        }
        loop {
            if self.ahead.is_none() {
                self.ahead = self.next()?;
            }
            match &self.ahead {
                Some((pts, _)) if *pts <= limit => self.current = self.ahead.take(),
                _ => break,
            }
        }
        // Before the first frame, the first frame shows.
        let shown = self
            .current
            .as_ref()
            .or(self.ahead.as_ref())
            .map(|(_, frame)| frame)
            .ok_or_else(|| format!("{}: frame {index} could not be read", self.path))?;
        let mut convert = match self.convert.take() {
            Some(convert) if convert.fits(shown, size, deep) => convert,
            _ => Convert::new(shown, size, deep)
                .map_err(|error| format!("{}: {error}", self.path))?,
        };
        // The filter takes the frame it is given and leaves it empty, so it
        // gets a copy: the frame read stays to be drawn again, at another
        // size or for another layer.
        let picture = convert
            .run(&shown.clone())
            .map_err(|error| format!("{}: {error}", self.path));
        self.convert = Some(convert);
        picture
    }

    fn seek(&mut self, pts: i64) -> Result<(), String> {
        let (num, den) = self.time_base;
        let micros = (i128::from(pts) * i128::from(num) * 1_000_000).div_euclid(i128::from(den));
        let micros = micros as i64;
        if self.input.seek(micros, ..micros).is_err() {
            // Some files cannot seek; read them again from the top.
            *self = Self::open(&self.path, self.rate)?;
            return Ok(());
        }
        self.decoder.flush();
        self.current = None;
        self.ahead = None;
        self.eof = false;
        Ok(())
    }

    /// The next frame in decode order with its timestamp, or `None` at the
    /// end.
    fn next(&mut self) -> Result<Option<(i64, frame::Video)>, String> {
        loop {
            let mut decoded = frame::Video::empty();
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    let pts = decoded.timestamp().unwrap_or_else(|| {
                        self.ahead
                            .as_ref()
                            .or(self.current.as_ref())
                            .map_or(self.first, |(pts, _)| pts + self.pts_of(1) - self.first)
                    });
                    return Ok(Some((pts, decoded)));
                }
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(error) if is_again(&error) => {}
                Err(error) => return Err(format!("{}: {error}", self.path)),
            }
            if self.eof {
                return Ok(None);
            }
            match self.input.packets().next() {
                Some((stream, packet)) if stream.index() == self.stream => {
                    match self.decoder.send_packet(&packet) {
                        Ok(()) => {}
                        Err(error) if is_again(&error) => {}
                        Err(ffmpeg::Error::Eof) => self.eof = true,
                        // A damaged packet: read on.
                        Err(_) => {}
                    }
                }
                Some(_) => {}
                None => {
                    self.eof = true;
                    let _ = self.decoder.send_eof();
                }
            }
        }
    }
}

fn is_again(error: &ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other { errno } if *errno == ffmpeg::util::error::EAGAIN)
}

/// A filter graph turning decoded frames of one format and size into RGBA
/// of another size.
struct Convert {
    graph: filter::Graph,
    input: (ffmpeg::format::Pixel, u32, u32),
    size: (u32, u32),
    deep: bool,
}

impl Convert {
    fn new(frame: &frame::Video, size: (u32, u32), deep: bool) -> Result<Self, ffmpeg::Error> {
        let (width, height) = (frame.width(), frame.height());
        let format = frame.format();
        let name = format
            .descriptor()
            .map(|descriptor| descriptor.name().to_owned())
            .filter(|name| !name.is_empty() && name != "none")
            .unwrap_or_else(|| "yuv420p".to_owned());
        let aspect = frame.aspect_ratio();
        let aspect = if aspect.numerator() <= 0 || aspect.denominator() <= 0 {
            (1, 1)
        } else {
            (aspect.numerator(), aspect.denominator())
        };
        let args = format!(
            "video_size={width}x{height}:pix_fmt={name}:time_base=1/1:pixel_aspect={}/{}",
            aspect.0, aspect.1
        );
        let out = if deep { "rgba64le" } else { "rgba" };
        let spec = if (width, height) == size {
            format!("format={out}")
        } else {
            format!("scale=w={}:h={}:flags=bicubic,format={out}", size.0, size.1)
        };
        let mut graph = filter::Graph::new();
        let buffer = filter::find("buffer").ok_or(ffmpeg::Error::FilterNotFound)?;
        let sink = filter::find("buffersink").ok_or(ffmpeg::Error::FilterNotFound)?;
        graph.add(&buffer, "in", &args)?;
        graph.add(&sink, "out", "")?;
        graph.output("in", 0)?.input("out", 0)?.parse(&spec)?;
        graph.validate()?;
        Ok(Self {
            graph,
            input: (format, width, height),
            size,
            deep,
        })
    }

    fn fits(&self, frame: &frame::Video, size: (u32, u32), deep: bool) -> bool {
        self.input == (frame.format(), frame.width(), frame.height())
            && self.size == size
            && self.deep == deep
    }

    fn run(&mut self, frame: &frame::Video) -> Result<frame::Video, String> {
        let mut source = self
            .graph
            .get("in")
            .ok_or("the filter graph has no source")?;
        source
            .source()
            .add(frame)
            .map_err(|error| error.to_string())?;
        let mut sink = self
            .graph
            .get("out")
            .ok_or("the filter graph has no sink")?;
        let mut rgba = frame::Video::empty();
        sink.sink()
            .frame(&mut rgba)
            .map_err(|error| error.to_string())?;
        Ok(rgba)
    }
}

/// RGBA rows packed together, opaque.
fn packed(rgba: &frame::Video) -> Vec<u8> {
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let (stride, data) = (rgba.stride(0), rgba.data(0));
    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        out.extend_from_slice(&data[y * stride..y * stride + width * 4]);
    }
    for pixel in out.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(source_in: i64, speed: &str, duration: i64) -> ClipSource {
        ClipSource {
            path: String::new(),
            rate: Rate::FPS_24,
            frames: 240,
            width: 16,
            height: 9,
            alpha: false,
            deep: None,
            source_in: Rate::FPS_24.time_of(source_in),
            speed: Ratio::parse(speed).unwrap(),
            duration: Rate::FPS_24.time_of(duration),
        }
    }

    fn at(frames: i64, rate: Rate) -> Time {
        rate.time_of(frames)
    }

    #[test]
    fn clips_show_their_source_frame_by_frame() {
        let forward = clip(10, "1", 48);
        assert_eq!(forward.frame_at_time(at(0, Rate::FPS_24)), 10);
        assert_eq!(forward.frame_at_time(at(5, Rate::FPS_24)), 15);
        // Handles before the clip show the frames before its source in.
        assert_eq!(forward.frame_at_time(at(-3, Rate::FPS_24)), 7);
        let fast = clip(10, "2", 24);
        assert_eq!(fast.frame_at_time(at(5, Rate::FPS_24)), 20);
        let slow = clip(10, "1/2", 48);
        assert_eq!(slow.frame_at_time(at(5, Rate::FPS_24)), 12);
    }

    #[test]
    fn reversed_clips_run_back_from_their_source_out() {
        // Source [10, 58): the first frame is 57.
        let reverse = clip(10, "-1", 48);
        assert_eq!(reverse.frame_at_time(at(0, Rate::FPS_24)), 57);
        assert_eq!(reverse.frame_at_time(at(47, Rate::FPS_24)), 10);
        let fast = clip(10, "-2", 24);
        assert_eq!(fast.frame_at_time(at(0, Rate::FPS_24)), 57);
        assert_eq!(fast.frame_at_time(at(1, Rate::FPS_24)), 55);
    }

    #[test]
    fn other_rates_take_the_frame_showing() {
        // 30 fps media in a 24 fps sequence: frame 1 of the sequence is at
        // 1.25 media frames.
        let clip = ClipSource {
            rate: Rate::FPS_30,
            ..clip(0, "1", 48)
        };
        assert_eq!(clip.frame_at_time(at(1, Rate::FPS_24)), 1);
        assert_eq!(clip.frame_at_time(at(4, Rate::FPS_24)), 5);
        assert_eq!(frame_count(Rate::FPS_30, Rate::FPS_24.time_of(1)), 2);
    }
}
