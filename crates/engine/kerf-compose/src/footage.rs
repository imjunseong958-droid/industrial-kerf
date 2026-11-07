//! Footage for the renderer: the frames each media clip shows, decoded by
//! libav.
//!
//! Every media clip is a footage item of its own, and its layer's time is
//! clip time, so the frame a clip shows is worked out here, exactly, from
//! the clip's source in and speed. A frame is the one whose display starts
//! at or before that source time, as a cut shows it. Decoders keep their
//! place in a file and read on to the next frame; they seek only to go
//! back further than the frames they keep, or far ahead. A frame just after
//! or just before one being read waits for that decoder rather than opening
//! another, so frames drawn on several threads, which finish in any order,
//! still decode each file once. Frames are converted to RGBA by
//! libavfilter, as the graph converts them, and kept in a bounded cache;
//! a frame for the GPU at its full size stays in its planes for the GPU to
//! convert, at less than half the bytes ([`yuv_of`]).
//! A clip never shown at its full size is converted at the size it is
//! shown, as the graph scales it. [`Pictures`] gives them to the compositor
//! in linear light.

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::context::Input;
use ffmpeg_next::{decoder, filter, frame, media};
use kerf_color::{ColorPipeline, ColorSpace, InputTransform, Range};
use kerf_draw::Image as Picture;
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Rate, Retime, Speed, Time};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Decoders kept open across reads.
const MAX_DECODERS: usize = 8;

/// How long a decoder is kept with nothing reading from it. Each holds the frames of its threads, about
/// 300 MB at 4K, and one left behind by a seek would otherwise hold them for good.
const DECODER_IDLE: Duration = Duration::from_secs(5);

/// How many bytes of frames as they decoded are kept, for a frame drawn again, as after an edit. A
/// dozen frames at 4K, sixty at 1000x562. Going forward they are not asked for twice.
const DECODED_BUDGET: usize = 128 << 20;

/// The frames a decoder keeps behind the one it last read. Frames drawn on several threads ask a
/// little out of order, by about how many are drawn at once, and with nothing kept each of those
/// sent another decoder through every frame of the file.
const KEPT_BEHIND: usize = 6;

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

/// A picture as the flow reads it.
fn raster(picture: &Picture) -> kerf_raster::Image {
    kerf_raster::Image {
        width: picture.w as u32,
        height: picture.h as u32,
        data: picture
            .px
            .chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
            .collect(),
    }
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
    pub retime: Retime,
    pub source_in: Time,
    pub speed: Speed,
    pub duration: Time,
}

impl ClipSource {
    /// The media frame shown at clip time `at`: the one showing at its
    /// source time, which a reversed clip runs back from its source out.
    pub fn frame_at_time(&self, at: Time) -> i64 {
        let units = |time: Time| i128::from(time.flicks());
        // Source flicks, as a fraction over the clip speed's denominator: one ratio stays exact,
        // and a ramp is exact to the flick.
        let (forward, reverse, den) = match self.speed.constant() {
            Some(ratio) => {
                let (num, den) = (
                    i128::from(ratio.num().unsigned_abs()),
                    i128::from(ratio.den()),
                );
                (units(at) * num, units(at) * num, den)
            }
            None => {
                let played = units(self.speed.played(at));
                (played, played, 1)
            }
        };
        // Frames per unit.
        let per = i128::from(FLICKS_PER_SECOND) * i128::from(self.rate.den()) * den;
        let frame = if self.speed.is_reverse() {
            // The frame showing just before the source position.
            let out = units(self.source_in + self.speed.played(self.duration));
            let position = (out * den - reverse) * i128::from(self.rate.num());
            -(-position).div_euclid(per) - 1
        } else {
            let position = (units(self.source_in) * den + forward) * i128::from(self.rate.num());
            position.div_euclid(per)
        };
        frame.clamp(0, i128::from(self.frames.max(1) - 1)) as i64
    }

    /// The media frame clip time `at` lies after and how far on toward the next, in 64ths, when
    /// it lies between two frames.
    fn between_at(&self, at: Time) -> Option<(i64, u16)> {
        let seconds = |time: Time| time.flicks() as f64 / FLICKS_PER_SECOND as f64;
        let per_second = f64::from(self.rate.num()) / f64::from(self.rate.den());
        let played = self.speed.played_seconds(seconds(at));
        let position = if self.speed.is_reverse() {
            let out = self.source_in + self.speed.played(self.duration);
            (seconds(out) - played) * per_second - 1.0
        } else {
            (seconds(self.source_in) + played) * per_second
        };
        let first = position.floor();
        let between = ((position - first) * 64.0).round() as u16;
        let first = first as i64;
        if between == 0 || between == 64 || first < 0 || first + 1 >= self.frames {
            return None;
        }
        Some((first, between))
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

    /// Frame `index` of `clip`'s file for the GPU at `size`: as it decoded if the GPU can take it so,
    /// or else as [`Self::picture`] makes it.
    fn for_gpu(
        &self,
        clip: &ClipSource,
        index: i64,
        size: (u32, u32),
    ) -> Option<Arc<frame::Video>> {
        let (loan, mut decoder) = match self.checkout(clip, index) {
            Ok(checked_out) => checked_out,
            Err(problem) => {
                lock(&self.problems).insert(clip.path.clone(), problem);
                return None;
            }
        };
        let read = decoder.shown(index).cloned().and_then(|shown| {
            if !clip.alpha && (shown.width(), shown.height()) == size && yuv_of(&shown).is_some() {
                Ok(shown)
            } else {
                decoder
                    .picture(index, size, clip.deep.is_some())
                    .map(Arc::new)
            }
        });
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
                        .filter(|position| reads_on(*position) || decoder.keeps(index))
                        .map(|position| (at, position))
                })
                .max_by_key(|(_, position)| *position)
                .map(|(at, _)| at);
            if let Some(at) = free {
                let decoder = decoders.idle.remove(at);
                return Ok((decoders.lend(&clip.path, index), decoder));
            }
            // One being read to around it has it or soon will, whichever finishes first.
            let coming = decoders.busy.iter().any(|(_, path, to)| {
                *path == clip.path
                    && (reads_on(*to) || (index..=index + KEPT_BEHIND as i64).contains(to))
            });
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
        decoders.idle.push(Decoder {
            returned: Instant::now(),
            ..decoder
        });
        if decoders.idle.len() > MAX_DECODERS {
            decoders.idle.remove(0);
        }
        decoders
            .idle
            .retain(|decoder| decoder.returned.elapsed() < DECODER_IDLE);
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
            decoded: Mutex::new(FrameCache::new(DECODED_BUDGET, decoded_bytes)),
        }
    }

    /// Files that could not be read, with why.
    pub fn problems(&self) -> Vec<(String, String)> {
        self.pool.problems()
    }

    /// The frame clip `index` shows at clip time `at`, decoded at `size`.
    pub fn picture(&self, index: usize, at: Time, size: (u32, u32)) -> Option<Arc<Picture>> {
        let clip = self.clips.get(index)?;
        let size = (size.0.max(1), size.1.max(1));
        if clip.retime != Retime::Frames {
            if let Some((first, between)) = clip.between_at(at) {
                return self.between(clip, first, between, size);
            }
        }
        self.whole(clip, clip.frame_at_time(at), size)
    }

    /// The picture a fraction of the way from one frame of the file to the next.
    fn between(
        &self,
        clip: &ClipSource,
        first: i64,
        between: u16,
        size: (u32, u32),
    ) -> Option<Arc<Picture>> {
        let key = (clip.path.clone(), first, size, between);
        if let Some(picture) = lock(&self.cache).get(&key) {
            return Some(picture);
        }
        let from = raster(&*self.whole(clip, first, size)?);
        let to = raster(&*self.whole(clip, first + 1, size)?);
        let along = f32::from(between) / 64.0;
        let made = match clip.retime {
            Retime::Flow => kerf_raster::flow::interpolate(&from, &to, along),
            _ => kerf_raster::flow::mix(&from, &to, along),
        };
        let mut picture = Picture::new(made.width as usize, made.height as usize);
        for (pixel, value) in picture.px.chunks_exact_mut(4).zip(&made.data) {
            pixel.copy_from_slice(value);
        }
        let picture = Arc::new(picture);
        lock(&self.cache).insert(key, picture.clone());
        Some(picture)
    }

    fn whole(&self, clip: &ClipSource, frame: i64, size: (u32, u32)) -> Option<Arc<Picture>> {
        let key = (clip.path.clone(), frame, size, 0);
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

    /// The frame clip `index` shows at clip time `at`, decoded at `size`, in its planes if the GPU can
    /// take it so ([`yuv_of`]) or else as the straight RGBA it decodes to, for the GPU to take into
    /// linear light.
    pub fn decoded(&self, index: usize, at: Time, size: (u32, u32)) -> Option<Decoded> {
        let clip = self.clips.get(index)?;
        let frame = clip.frame_at_time(at);
        let size = (size.0.max(1), size.1.max(1));
        let key = (clip.path.clone(), frame, size, 0);
        // Not held while decoding: other frames go on.
        let kept = lock(&self.decoded).get(&key);
        let picture = match kept {
            Some(picture) => picture,
            None => {
                let picture = self.pool.for_gpu(clip, frame, size)?;
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

/// A file, its frame, the size, and how far on toward the next frame.
type FrameKey = (String, i64, (u32, u32), u16);

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
    (0..decoded.planes())
        .map(|plane| decoded.stride(plane) * decoded.plane_height(plane) as usize)
        .sum()
}

/// `frame` as the GPU can take it in its planes, for a format and a color it knows how to make into
/// what libavfilter would. `None` for any other, which goes up as RGBA.
///
/// At 8 bits that is what comes out to the same bytes: a frame on the GPU is the same as the frame
/// with no effects that skips it, and a grade at 0 is not there. libswscale has a fast path for that
/// only on 64-bit ARM, for 4:2:0 and 4:2:2, at widths of 16 and even heights. Elsewhere it has other
/// ways, which the kernel does not go by. Deeper footage has no such frame to match, and comes within
/// 3 of 255.
pub(crate) fn yuv_of(frame: &frame::Video) -> Option<kerf_gpu::Yuv<'_>> {
    use ffmpeg::color::{Range as Levels, Space};
    use ffmpeg::format::Pixel;
    let ((across, down), bits, jpeg) = match frame.format() {
        Pixel::YUV420P => ((1, 1), 8, false),
        Pixel::YUV422P => ((1, 0), 8, false),
        Pixel::YUV444P => ((0, 0), 8, false),
        Pixel::YUVJ420P => ((1, 1), 8, true),
        Pixel::YUVJ422P => ((1, 0), 8, true),
        Pixel::YUVJ444P => ((0, 0), 8, true),
        Pixel::YUV420P10LE => ((1, 1), 10, false),
        Pixel::YUV422P10LE => ((1, 0), 10, false),
        Pixel::YUV444P10LE => ((0, 0), 10, false),
        Pixel::YUV420P12LE => ((1, 1), 12, false),
        Pixel::YUV422P12LE => ((1, 0), 12, false),
        Pixel::YUV444P12LE => ((0, 0), 12, false),
        _ => return None,
    };
    if bits == 8 {
        let fast_path = cfg!(target_arch = "aarch64")
            && across == 1
            && frame.width().is_multiple_of(16)
            && frame.height().is_multiple_of(1 << down);
        if !fast_path {
            return None;
        }
    }
    // As libswscale goes by a frame's tags: with none it takes BT.601.
    let matrix = match frame.color_space() {
        Space::BT709 => kerf_color::Matrix::Bt709,
        Space::BT2020NCL => kerf_color::Matrix::Bt2020Ncl,
        Space::Unspecified | Space::SMPTE170M | Space::BT470BG => kerf_color::Matrix::Bt601,
        _ => return None,
    };
    let stride = |plane: usize| u32::try_from(frame.stride(plane)).ok();
    Some(kerf_gpu::Yuv {
        planes: [frame.data(0), frame.data(1), frame.data(2)],
        strides: [stride(0)?, stride(1)?, stride(2)?],
        size: (frame.width(), frame.height()),
        chroma_shift: (across, down),
        bits,
        matrix,
        full_range: jpeg || frame.color_range() == Levels::JPEG,
        // libswscale repeats 4:2:0's chroma going to 8-bit RGBA, and filters it down going to 16-bit.
        // Across, it repeats it either way.
        chroma_smooth: bits > 8 && down > 0,
    })
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
    current: Option<(i64, Arc<frame::Video>)>,
    ahead: Option<(i64, Arc<frame::Video>)>,
    /// The frames read before `current`, oldest first, one after the other.
    behind: VecDeque<(i64, Arc<frame::Video>)>,
    /// Frames this decoder has been given by libav, to say what reading costs.
    read: u64,
    /// When it was last given back.
    returned: Instant,
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
            behind: VecDeque::new(),
            read: 0,
            returned: Instant::now(),
            eof: false,
            convert: None,
        })
    }

    /// The index of the last frame read, by the media's rate.
    fn position(&self) -> Option<i64> {
        self.current.as_ref().map(|(pts, _)| self.index_of(*pts))
    }

    /// Whether frame `index` is among those kept behind the last one read.
    fn keeps(&self, index: i64) -> bool {
        self.kept(index).is_some()
    }

    /// The frame kept behind the last one read that frame `index` shows: the last starting by its
    /// start, give or take half a frame, as [`Self::shown`] takes it, with a later one kept to
    /// show where it ends.
    fn kept(&self, index: i64) -> Option<&Arc<frame::Video>> {
        let limit = (self.pts_of(index) + self.pts_of(index + 1)) / 2;
        // Only behind: at the last one read or past it, reading goes on as ever.
        if self.current.as_ref().is_none_or(|(pts, _)| *pts <= limit) {
            return None;
        }
        self.behind
            .iter()
            .rev()
            .find(|(pts, _)| *pts <= limit)
            .map(|(_, frame)| frame)
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
        let shown = self.shown(index)?.clone();
        let mut convert = match self.convert.take() {
            Some(convert) if convert.fits(&shown, size, deep) => convert,
            _ => Convert::new(&shown, size, deep)
                .map_err(|error| format!("{}: {error}", self.path))?,
        };
        // The filter takes the frame it is given and leaves it empty, so it
        // gets a copy: the frame read stays to be drawn again, at another
        // size or for another layer.
        let picture = convert
            .run(&shown.as_ref().clone())
            .map_err(|error| format!("{}: {error}", self.path));
        self.convert = Some(convert);
        picture
    }

    /// Frame `index` as it decoded: the last frame starting by its start, give or take half a frame
    /// for timestamps that round.
    fn shown(&mut self, index: i64) -> Result<&Arc<frame::Video>, String> {
        if self.keeps(index) {
            return self
                .kept(index)
                .ok_or_else(|| format!("{}: frame {index} could not be read", self.path));
        }
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
                Some((pts, _)) if *pts <= limit => {
                    if let Some(passed) = self.current.take() {
                        if self.behind.len() == KEPT_BEHIND {
                            self.behind.pop_front();
                        }
                        self.behind.push_back(passed);
                    }
                    self.current = self.ahead.take();
                }
                _ => break,
            }
        }
        // Before the first frame, the first frame shows.
        self.current
            .as_ref()
            .or(self.ahead.as_ref())
            .map(|(_, frame)| frame)
            .ok_or_else(|| format!("{}: frame {index} could not be read", self.path))
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
        self.behind.clear();
        self.eof = false;
        Ok(())
    }

    /// The next frame in decode order with its timestamp, or `None` at the
    /// end.
    fn next(&mut self) -> Result<Option<(i64, Arc<frame::Video>)>, String> {
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
                    self.read += 1;
                    return Ok(Some((pts, Arc::new(decoded))));
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
            retime: Retime::Frames,
            source_in: Rate::FPS_24.time_of(source_in),
            speed: kerf_model::Ratio::parse(speed).unwrap().into(),
            duration: Rate::FPS_24.time_of(duration),
        }
    }

    fn at(frames: i64, rate: Rate) -> Time {
        rate.time_of(frames)
    }

    /// A small H.264 file with a keyframe every half second, or none where FFmpeg is not installed.
    fn encoded(directory: &std::path::Path) -> Option<String> {
        let path = directory.join("clip.mp4");
        let program = std::env::var("KERF_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_owned());
        let made = std::process::Command::new(program)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .args([
                "testsrc2=s=64x64:r=24:d=4",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "12",
            ])
            .arg(&path)
            .status();
        if !made.is_ok_and(|status| status.success()) {
            eprintln!("skipped: no ffmpeg to make a file with");
            return None;
        }
        Some(path.to_string_lossy().into_owned())
    }

    fn file_clip(path: &str) -> ClipSource {
        ClipSource {
            path: path.to_owned(),
            frames: 96,
            width: 64,
            height: 64,
            ..clip(0, "1", 96)
        }
    }

    /// The order three drawers finish three frames at a time in, when the second is the quickest.
    fn drawers_order(frames: i64) -> impl Iterator<Item = i64> {
        (0..frames / 3).flat_map(|at| [3 * at + 1, 3 * at, 3 * at + 2])
    }

    #[test]
    fn frames_asked_a_little_out_of_order_are_read_once_and_are_the_same_frames() {
        let directory = tempfile::tempdir().unwrap();
        let Some(path) = encoded(directory.path()) else {
            return;
        };
        let mut in_order = Decoder::open(&path, Rate::FPS_24).unwrap();
        let frames: Vec<Vec<u8>> = (0..90)
            .map(|index| in_order.shown(index).unwrap().data(0).to_vec())
            .collect();
        let mut decoder = Decoder::open(&path, Rate::FPS_24).unwrap();
        for index in drawers_order(90) {
            let shown = decoder.shown(index).unwrap().data(0).to_vec();
            assert!(shown == frames[index as usize], "frame {index}");
        }
        // It read each frame once, and the one past where it stopped. Going back for each would have
        // read a half-second's frames again every time.
        assert!(
            decoder.read <= in_order.read + 2,
            "read {} and in order {}",
            decoder.read,
            in_order.read
        );
        // Further back than it keeps, it goes back and reads, and it is the same frame.
        let before = decoder.read;
        let shown = decoder.shown(40).unwrap().data(0).to_vec();
        assert!(shown == frames[40]);
        assert!(decoder.read > before);
    }

    #[test]
    fn frames_drawn_out_of_order_on_one_decoder_do_not_open_more() {
        let directory = tempfile::tempdir().unwrap();
        let Some(path) = encoded(directory.path()) else {
            return;
        };
        let clip = file_clip(&path);
        let pool = DecoderPool::new();
        for index in drawers_order(90) {
            assert!(
                pool.picture(&clip, index, (64, 64)).is_some(),
                "frame {index}"
            );
        }
        let decoders = lock(&pool.decoders);
        assert_eq!(decoders.idle.len(), 1, "one decoder is enough");
        assert!(
            decoders.idle[0].read <= 92,
            "read {} frames for 90",
            decoders.idle[0].read
        );
    }

    #[test]
    fn a_decoder_left_idle_lets_go_of_its_frames() {
        let directory = tempfile::tempdir().unwrap();
        let Some(path) = encoded(directory.path()) else {
            return;
        };
        let clip = file_clip(&path);
        let pool = DecoderPool::new();
        let (first, left) = pool.checkout(&clip, 0).unwrap();
        let (second, kept) = pool.checkout(&clip, 90).unwrap();
        pool.give_back(first, left);
        assert_eq!(lock(&pool.decoders).idle.len(), 1);
        lock(&pool.decoders).idle[0].returned =
            Instant::now() - DECODER_IDLE - Duration::from_secs(1);
        // The one coming back stays, and the one long idle goes.
        pool.give_back(second, kept);
        assert_eq!(lock(&pool.decoders).idle.len(), 1);
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

    fn ramped(speed: kerf_model::Speed) -> ClipSource {
        ClipSource {
            speed,
            ..clip(10, "1", 48)
        }
    }

    fn keys(speeds: &[(i64, &str)]) -> kerf_model::Speed {
        kerf_model::Speed::ramp(
            speeds
                .iter()
                .map(|(frame, speed)| kerf_model::RampKey {
                    at: Rate::FPS_24.time_of(*frame),
                    speed: kerf_model::Ratio::parse(speed).unwrap(),
                    interp: kerf_model::Interp::Linear,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn a_ramp_shows_the_frame_its_area_has_reached() {
        // From 2 to 1/2 in a line over 24 frames plays 30 frames.
        let clip = ramped(keys(&[(0, "2"), (24, "1/2")]));
        assert_eq!(clip.frame_at_time(at(0, Rate::FPS_24)), 10);
        assert_eq!(clip.frame_at_time(at(24, Rate::FPS_24)), 40);
        // Half speed from then on.
        assert_eq!(clip.frame_at_time(at(28, Rate::FPS_24)), 42);
        // Half way along, the speed is 5/4 and the area (2 + 5/4) / 2 × 12 = 19.5.
        assert_eq!(clip.frame_at_time(at(12, Rate::FPS_24)), 29);
        assert_eq!(clip.between_at(at(12, Rate::FPS_24)), Some((29, 32)));
    }

    #[test]
    fn a_reversed_ramp_runs_back_from_its_source_out() {
        // Over 24 frames from -1 to -1/2 plays 18 frames, then 12 more over 24 at -1/2: 30 in all.
        let clip = ClipSource {
            duration: Rate::FPS_24.time_of(48),
            ..ramped(keys(&[(0, "-1"), (24, "-1/2")]))
        };
        assert_eq!(clip.frame_at_time(at(0, Rate::FPS_24)), 39);
        assert_eq!(clip.frame_at_time(at(24, Rate::FPS_24)), 21);
        assert_eq!(clip.frame_at_time(at(47, Rate::FPS_24)), 10);
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
