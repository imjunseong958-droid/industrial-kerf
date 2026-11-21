//! What each media looks and sounds like, for views to paint: a poster (a
//! representative frame), a filmstrip (frames along the source, at a small
//! height) and its sound's peaks (the lowest and highest sample and the RMS
//! of every 5 ms, with coarser levels), as Qt's `MediaArtwork` made them.
//!
//! FFmpeg makes them on worker threads, never on the window's: posters
//! first so the Media band fills fast, then peaks, then filmstrips, which
//! are asked for only when a view wants one. Each is kept per media and
//! size under a key of the media's id, file and modification time, so a
//! relink, a replace or a new take makes it again, and media taken out of
//! the project lets go of its artwork. A file that is missing or cannot be
//! decoded has none, and the view draws its stand-in; nothing here panics
//! on what a file holds. FFmpeg is looked for where kerfd looks
//! ([`ffmpeg_bin`]: `KERF_FFMPEG`, beside the editor, the PATH).
//!
//! [`crate::session::Session`] holds one [`Artwork`] (`session.artwork`),
//! tells it the project's media on each snapshot, and redraws the window
//! when something new has come. A view asks it while it renders:
//!
//! - [`Artwork::poster`]: the media's poster, as a BGRA [`RenderImage`]
//!   to paint fitted (the Media band's tiles, the Inspector).
//! - [`Artwork::frame_at`]: the filmstrip frame nearest a second of the
//!   source at about a height (a timeline clip's tiles, the poster standing
//!   in until the strip comes); [`paint_filmstrip`] paints a clip's tiles.
//! - [`Artwork::peaks`]: the sound's [`Peaks`]; [`Peaks::range`] gives the
//!   extremes and RMS of any span, and [`paint_waveform`] paints them.
//!
//! Asking is cheap: what is not made yet is queued once and answered None
//! until it comes.

use crate::model::Source;
use gpui::{
    point, px, Bounds, ContentMask, Corners, PathBuilder, Pixels, RenderImage, Rgba, Window,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

/// A poster covers a Media band tile at twice its size, so it is sharp
/// fitted there on a Retina screen and filled into the Inspector's.
pub const POSTER_SIZE: (u32, u32) = (248, 140);
/// Filmstrip frames are this tall unless a view asks for a smaller or a
/// larger strip ([`strip_height`]), and at most this many along a source.
pub const STRIP_HEIGHT: u32 = 96;
pub const STRIP_FRAMES: usize = 64;
/// Peaks are taken from the sound resampled to this rate, mixed to mono.
pub const PEAK_SAMPLE_RATE: u32 = 16_000;
/// Sources longer than this have their strip sought a frame at a time;
/// shorter ones are decoded once.
const DECODED_STRIP_SECONDS: f64 = 16.0;
/// How many FFmpegs run at once.
const WORKERS: usize = 2;

// ------------------------------------------------------------------ peaks

/// The extremes and RMS of a stretch of sound, samples -1..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bucket {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
}

impl Bucket {
    /// Several buckets as one: their extremes, and the RMS of them all.
    pub fn merge(buckets: &[Bucket]) -> Bucket {
        let Some(first) = buckets.first() else {
            return Bucket::default();
        };
        let mut merged = *first;
        let mut squares = 0.0f64;
        for bucket in buckets {
            merged.min = merged.min.min(bucket.min);
            merged.max = merged.max.max(bucket.max);
            squares += f64::from(bucket.rms) * f64::from(bucket.rms);
        }
        merged.rms = (squares / buckets.len() as f64).sqrt() as f32;
        merged
    }
}

/// A source's sound as data, not a picture: a [`Bucket`] for every 5 ms,
/// and coarser levels each merging four of the one below, so a span of
/// minutes reads a handful of buckets and a waveform costs what its
/// pixels do, not what its length does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Peaks {
    levels: Vec<Vec<Bucket>>,
}

impl Peaks {
    pub const BUCKETS_PER_SECOND: u32 = 200;
    pub const MERGE: usize = 4;

    /// Peaks from their finest buckets, the coarser levels made from them.
    pub fn from_buckets(buckets: Vec<Bucket>) -> Self {
        if buckets.is_empty() {
            return Self::default();
        }
        let mut levels = vec![buckets];
        while let Some(finer) = levels.last().filter(|finer| finer.len() > 1) {
            let coarser = finer.chunks(Self::MERGE).map(Bucket::merge).collect();
            levels.push(coarser);
        }
        Self { levels }
    }

    pub fn is_empty(&self) -> bool {
        self.levels.first().is_none_or(Vec::is_empty)
    }

    /// Its levels, the finest first.
    pub fn levels(&self) -> &[Vec<Bucket>] {
        &self.levels
    }

    /// How long the sound is, in seconds.
    pub fn duration(&self) -> f64 {
        self.levels
            .first()
            .map_or(0.0, |finest| finest.len() as f64)
            / f64::from(Self::BUCKETS_PER_SECOND)
    }

    /// Seconds a bucket of `level` spans.
    pub fn bucket_seconds(level: usize) -> f64 {
        (Self::MERGE as f64).powi(level as i32) / f64::from(Self::BUCKETS_PER_SECOND)
    }

    /// The extremes and RMS from `from` to `to` seconds (either way round),
    /// from the coarsest level whose buckets still fit the span. A span
    /// shorter than a bucket (a deep zoom) is read between the buckets
    /// either side, so it draws a curve rather than steps. Outside the
    /// sound it is silence.
    pub fn range(&self, from: f64, to: f64) -> Bucket {
        if self.is_empty() || !from.is_finite() || !to.is_finite() {
            return Bucket::default();
        }
        let (from, to) = if to < from { (to, from) } else { (from, to) };
        let span = to - from;
        let mut level = 0;
        while level + 1 < self.levels.len() && Self::bucket_seconds(level + 1) <= span {
            level += 1;
        }
        let buckets = &self.levels[level];
        let size = Self::bucket_seconds(level);
        if span < size {
            let at = (from + to) / 2.0 / size - 0.5;
            let floor = at.floor();
            let bucket_at = |index: f64| {
                if index >= 0.0 && index < buckets.len() as f64 {
                    buckets[index as usize]
                } else {
                    Bucket::default()
                }
            };
            let (before, after) = (bucket_at(floor), bucket_at(floor + 1.0));
            let weight = (at - floor) as f32;
            let mix = |a: f32, b: f32| a + (b - a) * weight;
            return Bucket {
                min: mix(before.min, after.min),
                max: mix(before.max, after.max),
                rms: mix(before.rms, after.rms),
            };
        }
        let first = (from / size).floor().max(0.0) as usize;
        let last = ((to / size).ceil().max(0.0) as usize).min(buckets.len());
        if first < last {
            Bucket::merge(&buckets[first..last])
        } else {
            Bucket::default()
        }
    }

    /// The largest sample's size in the whole sound.
    pub fn loudest(&self) -> f64 {
        let Some(whole) = self.levels.last() else {
            return 0.0;
        };
        let all = Bucket::merge(whole);
        f64::from(all.min.abs().max(all.max.abs()))
    }

    /// How far a waveform of it is raised to fill its clip, as Logic's and
    /// Final Cut's are: its loudest peak reaches 95% of the height, a quiet
    /// recording raised by at most 12 dB.
    pub fn display_gain(&self) -> f64 {
        let peak = self.loudest();
        if peak > 1e-4 {
            (0.95 / peak).min(4.0)
        } else {
            1.0
        }
    }
}

/// Turns a stream of mono samples into the finest buckets a chunk at a
/// time, so an hour of sound never sits in memory as samples.
#[derive(Clone, Debug)]
pub struct PeakReducer {
    per_bucket: usize,
    count: usize,
    min: f32,
    max: f32,
    squares: f64,
    buckets: Vec<Bucket>,
}

impl PeakReducer {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            per_bucket: (sample_rate / Peaks::BUCKETS_PER_SECOND).max(1) as usize,
            count: 0,
            min: 0.0,
            max: 0.0,
            squares: 0.0,
            buckets: Vec::new(),
        }
    }

    pub fn add(&mut self, samples: &[f32]) {
        for &sample in samples {
            // A sample that is no number counts as silence.
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            if self.count == 0 {
                self.min = sample;
                self.max = sample;
            } else {
                self.min = self.min.min(sample);
                self.max = self.max.max(sample);
            }
            self.squares += f64::from(sample) * f64::from(sample);
            self.count += 1;
            if self.count == self.per_bucket {
                self.flush();
            }
        }
    }

    /// Takes little-endian f32 samples from `bytes`, keeping a partial
    /// sample for the next call in `rest`.
    pub fn add_bytes(&mut self, rest: &mut Vec<u8>, bytes: &[u8]) {
        rest.extend_from_slice(bytes);
        let whole = rest.len() / 4 * 4;
        let samples: Vec<f32> = rest[..whole]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|four| f32::from_le_bytes(*four))
            .collect();
        self.add(&samples);
        rest.drain(..whole);
    }

    fn flush(&mut self) {
        self.buckets.push(Bucket {
            min: self.min,
            max: self.max,
            rms: (self.squares / self.count as f64).sqrt() as f32,
        });
        self.count = 0;
        self.squares = 0.0;
    }

    /// The last, partial bucket, then the coarser levels.
    pub fn finish(mut self) -> Peaks {
        if self.count > 0 {
            self.flush();
        }
        Peaks::from_buckets(self.buckets)
    }
}

/// How far a waveform of `peaks` drawn at clip gain `gain` (linear) is
/// raised: to fill the clip ([`Peaks::display_gain`]) times the gain, but
/// never so a peak that does not truly go over full scale is pushed past
/// the top: a quiet take at +6 dB is drawn to fill the clip, not cut flat.
pub fn waveform_raise(peaks: &Peaks, gain: f64) -> f64 {
    let gain = if gain.is_finite() && gain > 0.0 {
        gain
    } else {
        1.0
    };
    let mut raise = gain * peaks.display_gain();
    let loudest = peaks.loudest();
    if loudest > 1e-4 && loudest * raise > 0.95 {
        raise = gain.max(0.95 / loudest);
    }
    raise
}

/// A sample's height in a waveform raised by `raise`, -1..1: on a gentle
/// curve between linear and the square root, which keeps the attacks and
/// the gaps between words an editor cuts on, and lets speech still read.
pub fn waveform_level(value: f32, raise: f64) -> f64 {
    let level = (f64::from(value) * raise).clamp(-1.0, 1.0);
    level.abs().powf(0.75).copysign(level)
}

/// One column of a waveform: the envelope's top and bottom and the RMS's,
/// -1..1 upwards from the centre line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Column {
    pub high: f64,
    pub low: f64,
    pub body_high: f64,
    pub body_low: f64,
}

/// The waveform of `peaks` from `from` to `to` seconds of the source in
/// `columns` columns (`to` before `from` reads the sound backwards),
/// raised by `raise` ([`waveform_raise`]).
pub fn waveform_columns(
    peaks: &Peaks,
    from: f64,
    to: f64,
    columns: usize,
    raise: f64,
) -> Vec<Column> {
    if columns == 0 {
        return Vec::new();
    }
    let per = (to - from) / columns as f64;
    (0..columns)
        .map(|column| {
            let seconds = from + column as f64 * per;
            let bucket = peaks.range(seconds, seconds + per);
            let high = waveform_level(bucket.max, raise);
            let low = waveform_level(bucket.min, raise);
            let rms = waveform_level(bucket.rms, raise);
            Column {
                high,
                low,
                body_high: rms.min(high),
                body_low: (-rms).max(low),
            }
        })
        .collect()
}

/// The most columns painted as one path (a path's points are counted in
/// 16 bits).
const COLUMNS_PER_PATH: usize = 2048;

/// Paints `peaks` from `from` to `to` seconds of the source across
/// `bounds`, a column for each pixel of the screen and only where the
/// content mask lets it show: the envelope of the extremes in `peak`, the
/// RMS inside it in `body`, either side of the centre line, at clip gain
/// `gain` (linear; 1 for none). `to` before `from` draws it reversed.
// Painting takes the window, the bounds and each measure it draws with.
#[allow(clippy::too_many_arguments)]
pub fn paint_waveform(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    peaks: &Peaks,
    from: f64,
    to: f64,
    peak: Rgba,
    body: Rgba,
    gain: f64,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if peaks.is_empty() || width <= 0.0 || height <= 0.0 {
        return;
    }
    let mask = window.content_mask().bounds;
    let ratio = window.scale_factor().max(1.0);
    let step = 1.0 / ratio;
    let left_edge = f32::from(bounds.origin.x);
    let mut left = left_edge.max(f32::from(mask.origin.x));
    let right = (left_edge + width).min(f32::from(mask.origin.x + mask.size.width) + 1.0);
    left = ((left * ratio).floor() / ratio).max(left_edge);
    if right <= left {
        return;
    }
    let per_x = (to - from) / f64::from(width);
    let columns = ((right - left) * ratio).ceil() as usize;
    let raise = waveform_raise(peaks, gain);
    let start = from + f64::from(left - left_edge) * per_x;
    let end = start + columns as f64 * f64::from(step) * per_x;
    let shape = waveform_columns(peaks, start, end, columns, raise);
    let centre = f32::from(bounds.origin.y) + height / 2.0;
    let half = height / 2.0;
    for (chunk, columns) in shape.chunks(COLUMNS_PER_PATH).enumerate() {
        let first = left + (chunk * COLUMNS_PER_PATH) as f32 * step;
        let place = Placing {
            first,
            step,
            centre,
            half,
        };
        if let Some(path) = place.outline(columns, |column| (column.high, column.low)) {
            window.paint_path(path, peak);
        }
        if let Some(path) = place.outline(columns, |column| (column.body_high, column.body_low)) {
            window.paint_path(path, body);
        }
    }
}

/// Where a run of a waveform's columns goes on the screen.
struct Placing {
    first: f32,
    step: f32,
    centre: f32,
    half: f32,
}

impl Placing {
    /// The shape between each column's top and bottom (`edges`), out to
    /// the columns' outer edges.
    fn outline(
        &self,
        columns: &[Column],
        edges: impl Fn(&Column) -> (f64, f64),
    ) -> Option<gpui::Path<Pixels>> {
        let (first_column, last_column) = (columns.first()?, columns.last()?);
        let y = |level: f64| px(self.centre - level as f32 * self.half);
        let x = |index: usize| px(self.first + (index as f32 + 0.5) * self.step);
        let end = px(self.first + columns.len() as f32 * self.step);
        let mut path = PathBuilder::fill();
        path.move_to(point(px(self.first), y(edges(first_column).0)));
        for (index, column) in columns.iter().enumerate() {
            path.line_to(point(x(index), y(edges(column).0)));
        }
        path.line_to(point(end, y(edges(last_column).0)));
        path.line_to(point(end, y(edges(last_column).1)));
        for (index, column) in columns.iter().enumerate().rev() {
            path.line_to(point(x(index), y(edges(column).1)));
        }
        path.line_to(point(px(self.first), y(edges(first_column).1)));
        path.close();
        path.build().ok()
    }
}

// -------------------------------------------------------------- pictures

/// Frames sampled evenly along a source at their own shape: frame `k`
/// shows the source `k * interval` seconds in.
#[derive(Clone)]
pub struct Filmstrip {
    pub frames: Vec<Arc<RenderImage>>,
    pub interval: f64,
    /// Width and height of its frames, in pixels.
    pub size: (u32, u32),
}

impl Filmstrip {
    /// The frame nearest `seconds` into the source.
    pub fn frame_index(&self, seconds: f64) -> Option<usize> {
        frame_index(self.frames.len(), self.interval, seconds)
    }

    /// Width over height of its pictures.
    pub fn aspect(&self) -> f32 {
        if self.size.1 == 0 {
            16.0 / 9.0
        } else {
            self.size.0 as f32 / self.size.1 as f32
        }
    }
}

/// Of `count` frames `interval` seconds apart, the one nearest `seconds`.
pub fn frame_index(count: usize, interval: f64, seconds: f64) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if interval.is_nan() || interval <= 0.0 || !seconds.is_finite() {
        return Some(0);
    }
    let index = (seconds / interval).round().clamp(0.0, (count - 1) as f64);
    Some(index as usize)
}

/// How many frames a strip of a `duration` second source takes, spread
/// evenly along it: eight a second covers the closest zoom's tiles, and a
/// long source shares its frames out more thinly.
pub fn strip_frame_count(duration: f64) -> usize {
    if duration.is_nan() || duration <= 0.0 {
        return STRIP_FRAMES;
    }
    ((duration * 8.0).ceil() as usize).clamp(1, STRIP_FRAMES)
}

/// The strip height a view asking for `height` pixels gets: 48, 96 or
/// 192, so a few sizes serve every zoom.
pub fn strip_height(height: f32) -> u32 {
    if height <= 48.0 {
        48
    } else if height <= STRIP_HEIGHT as f32 {
        STRIP_HEIGHT
    } else {
        192
    }
}

/// One picture FFmpeg wrote, as RGBA rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The PAM pictures (`-c:v pam`) in `stream`, back to back as FFmpeg's
/// image2pipe writes them: RGB, RGB with alpha, grey or grey with alpha at
/// 8 bits, made RGBA. Reading stops at the first one that does not read.
pub fn parse_pam_stream(stream: &[u8]) -> Vec<Picture> {
    let mut pictures = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        match parse_pam(&stream[at..]) {
            Some((picture, used)) => {
                pictures.push(picture);
                at += used;
            }
            None => break,
        }
    }
    pictures
}

/// One PAM picture at the start of `bytes`, and how many bytes it took.
fn parse_pam(bytes: &[u8]) -> Option<(Picture, usize)> {
    let header_end = bytes.windows(7).position(|window| window == b"ENDHDR\n")? + 7;
    let header = std::str::from_utf8(&bytes[..header_end]).ok()?;
    let mut lines = header.lines();
    if lines.next()?.trim() != "P7" {
        return None;
    }
    let (mut width, mut height, mut depth, mut maxval) = (0u32, 0u32, 0u32, 0u32);
    for line in lines {
        let mut words = line.split_whitespace();
        let (Some(key), value) = (words.next(), words.next()) else {
            continue;
        };
        let number = || value.and_then(|value| value.parse::<u32>().ok());
        match key {
            "WIDTH" => width = number()?,
            "HEIGHT" => height = number()?,
            "DEPTH" => depth = number()?,
            "MAXVAL" => maxval = number()?,
            _ => {}
        }
    }
    if width == 0 || height == 0 || maxval != 255 || !(1..=4).contains(&depth) {
        return None;
    }
    let pixels = (width as usize).checked_mul(height as usize)?;
    let size = pixels.checked_mul(depth as usize)?;
    let body = bytes.get(header_end..header_end.checked_add(size)?)?;
    let mut rgba = Vec::with_capacity(pixels * 4);
    for pixel in body.chunks_exact(depth as usize) {
        match depth {
            1 => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            2 => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            3 => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
            _ => rgba.extend_from_slice(pixel),
        }
    }
    Some((
        Picture {
            width,
            height,
            rgba,
        },
        header_end + size,
    ))
}

/// A picture as GPUI paints it (its textures take BGRA).
pub fn render_image(picture: &Picture) -> Option<Arc<RenderImage>> {
    crate::session::render_image(&picture.rgba, picture.width, picture.height)
}

/// Whether an FFmpeg pixel format carries alpha (yuva420p, rgba, gbrap…).
pub fn pixel_format_has_alpha(format: &str) -> bool {
    let format = format.to_ascii_lowercase();
    !format.is_empty()
        && (format.starts_with("yuva")
            || format.starts_with("ya")
            || format.starts_with("gbrap")
            || format.starts_with("ayuv")
            || ["rgba", "bgra", "argb", "abgr"]
                .iter()
                .any(|alpha| format.contains(alpha)))
}

/// Where `image` goes when fitted into `bounds` at its own shape: inside
/// them whole (`cover` false, bars either side), or covering them (`cover`
/// true, its edges given up).
pub fn fitted(bounds: Bounds<Pixels>, image: (u32, u32), cover: bool) -> Bounds<Pixels> {
    let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if image.0 == 0 || image.1 == 0 || width <= 0.0 || height <= 0.0 {
        return bounds;
    }
    let (across, down) = (width / image.0 as f32, height / image.1 as f32);
    let scale = if cover {
        across.max(down)
    } else {
        across.min(down)
    };
    let size = gpui::size(px(image.0 as f32 * scale), px(image.1 as f32 * scale));
    Bounds {
        origin: point(
            bounds.origin.x + (bounds.size.width - size.width) / 2.0,
            bounds.origin.y + (bounds.size.height - size.height) / 2.0,
        ),
        size,
    }
}

/// The size of `image`'s first frame, in pixels.
pub fn image_size(image: &RenderImage) -> (u32, u32) {
    let size = image.size(0);
    (size.width.0.max(0) as u32, size.height.0.max(0) as u32)
}

/// Paints `image` into `bounds` at its own shape, covering them (its edges
/// cut off) or inside them whole, with rounded corners `radius`.
pub fn paint_fitted(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    image: Arc<RenderImage>,
    cover: bool,
    radius: Pixels,
) {
    let placed = fitted(bounds, image_size(&image), cover);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        let _ = window.paint_image(placed, Corners::all(radius), image, 0, false);
    });
}

/// Paints a clip's filmstrip across `bounds`, from the clip's left edge:
/// 16:9 frames at its height with a 1 pt gap between them, each the frame
/// of `source` under its left edge (`source_at` turns seconds into the
/// clip into seconds of the source, [`crate::model::Clip::source_at`]),
/// filled rather than stretched, the poster standing in until the strip
/// comes. Only the tiles the content mask shows are painted. False when
/// there is nothing to paint yet (the clip draws its plain ground).
pub fn paint_filmstrip(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    artwork: &Artwork,
    source: &str,
    pixels_per_second: f32,
    source_at: impl Fn(f64) -> f64,
) -> bool {
    let height = f32::from(bounds.size.height);
    if height <= 0.0 || pixels_per_second <= 0.0 {
        return false;
    }
    let wanted = strip_height(height * window.scale_factor());
    if artwork.frame_at(source, 0.0, wanted as f32).is_none() {
        return false;
    }
    let tile = (height * 16.0 / 9.0).round().max(6.0);
    let step = tile + 1.0;
    let left = f32::from(bounds.origin.x);
    let mask = window.content_mask().bounds;
    let right =
        (left + f32::from(bounds.size.width)).min(f32::from(mask.origin.x + mask.size.width) + 1.0);
    let mut x = left + ((f32::from(mask.origin.x) - left) / step).floor().max(0.0) * step;
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        window.paint_quad(gpui::fill(bounds, gpui::black()));
        while x < right {
            let seconds = source_at(f64::from((x - left) / pixels_per_second));
            if let Some(image) = artwork.frame_at(source, seconds, wanted as f32) {
                let tile_bounds = Bounds {
                    origin: point(px(x), bounds.origin.y),
                    size: gpui::size(px(tile), bounds.size.height),
                };
                paint_fitted(window, tile_bounds, image, true, px(0.0));
            }
            x += step;
        }
    });
    true
}

// ---------------------------------------------------------------- the cache

/// What is known of a piece of artwork.
enum Slot<T> {
    /// Asked for, not come yet.
    Asked,
    Ready(T),
    /// The file is missing or would not decode: none, until the media's
    /// file changes.
    Failed,
}

impl<T: Clone> Slot<T> {
    fn ready(&self) -> Option<T> {
        match self {
            Slot::Ready(value) => Some(value.clone()),
            _ => None,
        }
    }
}

/// One media's artwork, under the key it was made for.
struct Entry {
    key: String,
    path: PathBuf,
    duration: f64,
    picture: bool,
    poster: Option<Slot<Arc<RenderImage>>>,
    peaks: Option<Slot<Arc<Peaks>>>,
    strips: HashMap<u32, Slot<Arc<Filmstrip>>>,
}

impl Entry {
    /// Every picture it holds, to let go of.
    fn images(&self) -> Vec<Arc<RenderImage>> {
        let mut images: Vec<Arc<RenderImage>> =
            self.poster.iter().filter_map(Slot::ready).collect();
        for strip in self.strips.values() {
            if let Some(strip) = strip.ready() {
                images.extend(strip.frames.iter().cloned());
            }
        }
        images
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Poster,
    Peaks,
    Strip,
}

/// A piece of artwork to make, and a long strip's frames so far.
struct Job {
    id: String,
    key: String,
    kind: Kind,
    path: PathBuf,
    duration: f64,
    height: u32,
    taken: Vec<Option<Picture>>,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    /// Drained in this order: posters, so the band fills fast, then peaks,
    /// then strips.
    queues: [VecDeque<Job>; 3],
    /// Pictures let go of, for the window to take out of its atlas.
    dropped: Vec<Arc<RenderImage>>,
    workers: usize,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    changed: AtomicBool,
    /// Tells the window that something has come ([`Artwork::wake_with`]).
    on_change: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Set when the artwork is dropped: workers end, and FFmpegs are stopped.
    stop: Arc<AtomicBool>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Every media's artwork for one project. Dropped, its FFmpegs are stopped.
pub struct Artwork {
    shared: Arc<Shared>,
    /// The directory relative paths are read against.
    directory: Option<PathBuf>,
}

impl Default for Artwork {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Drop for Artwork {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

/// When a media's file was last changed, in milliseconds; 0 when it is not there.
fn modified(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis())
}

impl Artwork {
    /// Artwork for the project in `directory` (None: nothing to read).
    /// Its workers start with its first job.
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                wake: Condvar::new(),
                changed: AtomicBool::new(false),
                on_change: OnceLock::new(),
                stop: Arc::new(AtomicBool::new(false)),
            }),
            directory,
        }
    }

    /// Follows the project's media (each snapshot's sources): media new to
    /// it, or whose file is another or has changed since, has its poster and
    /// peaks asked for again; media gone lets go of its artwork, and its
    /// jobs still queued are skipped.
    pub fn sync(&self, sources: &[Source]) {
        let mut state = self.shared.lock();
        let mut present = HashSet::new();
        let mut queued = false;
        for source in sources {
            present.insert(source.id.clone());
            let path = if source.path.is_empty() {
                PathBuf::new()
            } else {
                let path = PathBuf::from(&source.path);
                match (&self.directory, path.is_absolute()) {
                    (Some(directory), false) => directory.join(path),
                    _ => path,
                }
            };
            let key = format!("{}\u{0}{}", path.display(), modified(&path));
            if state
                .entries
                .get(&source.id)
                .is_some_and(|entry| entry.key == key)
            {
                continue;
            }
            if let Some(old) = state.entries.remove(&source.id) {
                let images = old.images();
                state.dropped.extend(images);
            }
            let exists = !source.path.is_empty() && path.is_file();
            let picture = source.is_picture() || !source.has_audio;
            let mut entry = Entry {
                key: key.clone(),
                path: path.clone(),
                duration: source.duration,
                picture,
                poster: picture.then_some(Slot::Failed),
                peaks: source.has_audio.then_some(Slot::Failed),
                strips: HashMap::new(),
            };
            if exists {
                let job = |kind| Job {
                    id: source.id.clone(),
                    key: key.clone(),
                    kind,
                    path: path.clone(),
                    duration: source.duration,
                    height: STRIP_HEIGHT,
                    taken: Vec::new(),
                };
                if picture {
                    entry.poster = Some(Slot::Asked);
                    state.queues[0].push_back(job(Kind::Poster));
                }
                if source.has_audio {
                    entry.peaks = Some(Slot::Asked);
                    state.queues[1].push_back(job(Kind::Peaks));
                }
                queued = true;
            }
            state.entries.insert(source.id.clone(), entry);
        }
        let gone: Vec<String> = state
            .entries
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(entry) = state.entries.remove(&id) {
                let images = entry.images();
                state.dropped.extend(images);
                self.shared.changed.store(true, Ordering::Relaxed);
            }
        }
        if queued {
            self.start_workers(&mut state);
            self.shared.wake.notify_all();
        }
    }

    fn start_workers(&self, state: &mut State) {
        while state.workers < WORKERS {
            state.workers += 1;
            let shared = self.shared.clone();
            let started = std::thread::Builder::new()
                .name("kerf-artwork".into())
                .spawn(move || work(&shared));
            if started.is_err() {
                state.workers -= 1;
                break;
            }
        }
    }

    /// Media `id`'s poster: one representative frame at its own shape,
    /// about [`POSTER_SIZE`]. None while it is made, and for sound alone,
    /// a missing file or one that does not decode.
    pub fn poster(&self, id: &str) -> Option<Arc<RenderImage>> {
        self.shared.lock().entries.get(id)?.poster.as_ref()?.ready()
    }

    /// Media `id`'s sound as [`Peaks`]: None while they are made, and for
    /// a picture with no sound or a file that does not decode.
    pub fn peaks(&self, id: &str) -> Option<Arc<Peaks>> {
        self.shared.lock().entries.get(id)?.peaks.as_ref()?.ready()
    }

    /// The frame of media `id`'s filmstrip nearest `seconds` into it, at
    /// about `height` pixels ([`strip_height`]); its poster until the strip
    /// comes, which this asks for the first time. None with neither.
    pub fn frame_at(&self, id: &str, seconds: f64, height: f32) -> Option<Arc<RenderImage>> {
        let height = strip_height(height);
        let mut state = self.shared.lock();
        let entry = state.entries.get_mut(id)?;
        let poster = entry.poster.as_ref().and_then(Slot::ready);
        if !entry.picture {
            return None;
        }
        match entry.strips.get(&height) {
            Some(Slot::Ready(strip)) => {
                return strip
                    .frame_index(seconds)
                    .and_then(|index| strip.frames.get(index).cloned())
                    .or(poster);
            }
            Some(_) => return poster,
            None => {}
        }
        // Not asked for at this height yet: once the poster is there (a file
        // that decodes), the strip goes in behind posters and peaks.
        if !matches!(entry.poster, Some(Slot::Ready(_))) {
            if matches!(entry.poster, Some(Slot::Failed) | None) {
                entry.strips.insert(height, Slot::Failed);
            }
            return poster;
        }
        entry.strips.insert(height, Slot::Asked);
        let job = Job {
            id: id.to_owned(),
            key: entry.key.clone(),
            kind: Kind::Strip,
            path: entry.path.clone(),
            duration: entry.duration,
            height,
            taken: Vec::new(),
        };
        state.queues[2].push_back(job);
        self.start_workers(&mut state);
        drop(state);
        self.shared.wake.notify_all();
        poster
    }

    /// Has `on_change` called from the thread that made it each time
    /// something comes, for the window to ask [`Artwork::take_changed`]
    /// then. Only the first one given is kept.
    pub fn wake_with(&self, on_change: impl Fn() + Send + Sync + 'static) {
        let _ = self.shared.on_change.set(Box::new(on_change));
    }

    /// Whether anything has come or gone since this was last asked: the
    /// window draws again.
    pub fn take_changed(&self) -> bool {
        self.shared.changed.swap(false, Ordering::Relaxed)
    }

    /// The pictures let go of since this was last asked, for the window to
    /// take out of its atlas (`Window::drop_image`).
    pub fn take_dropped(&self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.shared.lock().dropped)
    }

    /// Whether any artwork is still being made or waits to be.
    pub fn busy(&self) -> bool {
        let state = self.shared.lock();
        state.queues.iter().any(|queue| !queue.is_empty())
            || state.entries.values().any(|entry| {
                matches!(entry.poster, Some(Slot::Asked))
                    || matches!(entry.peaks, Some(Slot::Asked))
                    || entry
                        .strips
                        .values()
                        .any(|strip| matches!(strip, Slot::Asked))
            })
    }
}

/// A worker: takes the most wanted job whose media still has the key it
/// was asked for under, makes it, and keeps what it made.
fn work(shared: &Shared) {
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                let next = state.queues.iter_mut().find_map(VecDeque::pop_front);
                match next {
                    Some(job) => {
                        let current = state
                            .entries
                            .get(&job.id)
                            .is_some_and(|entry| entry.key == job.key);
                        if current {
                            break job;
                        }
                    }
                    None => {
                        state = shared
                            .wake
                            .wait_timeout(state, Duration::from_millis(500))
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                    }
                }
            }
        };
        match job.kind {
            Kind::Poster => {
                let made = poster(shared, &job).and_then(|picture| render_image(&picture));
                keep(shared, &job, |entry| {
                    entry.poster = Some(made.map_or(Slot::Failed, Slot::Ready));
                });
            }
            Kind::Peaks => {
                let made = peaks(shared, &job).filter(|peaks| !peaks.is_empty());
                keep(shared, &job, |entry| {
                    entry.peaks =
                        Some(made.map_or(Slot::Failed, |peaks| Slot::Ready(Arc::new(peaks))));
                });
            }
            Kind::Strip => {
                if let Some((job, strip)) = strip(shared, job) {
                    keep(shared, &job, |entry| {
                        let slot = strip.map_or(Slot::Failed, |strip| Slot::Ready(Arc::new(strip)));
                        entry.strips.insert(job.height, slot);
                    });
                }
            }
        }
    }
}

/// Keeps what `job` made, if its media still has the key it was asked
/// under; else what was made is let go of, never having been drawn.
fn keep(shared: &Shared, job: &Job, store: impl FnOnce(&mut Entry)) {
    let mut state = shared.lock();
    if let Some(entry) = state
        .entries
        .get_mut(&job.id)
        .filter(|entry| entry.key == job.key)
    {
        store(entry);
        shared.changed.store(true, Ordering::Relaxed);
        drop(state);
        if let Some(on_change) = shared.on_change.get() {
            on_change();
        }
    }
}

/// Anamorphic pictures squared up first, at their display shape.
const SQUARE: &str = "scale=trunc(iw*sar/2)*2:ih,setsar=1";

fn quiet() -> Vec<String> {
    ["-nostdin", "-v", "error"].map(str::to_owned).to_vec()
}

fn pam_out() -> Vec<String> {
    [
        "-f",
        "image2pipe",
        "-c:v",
        "pam",
        "-pix_fmt",
        "rgba",
        "pipe:1",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn poster(shared: &Shared, job: &Job) -> Option<Picture> {
    let (width, height) = POSTER_SIZE;
    let mut args = quiet();
    args.extend([
        "-i".into(),
        job.path.to_string_lossy().into_owned(),
        "-an".into(),
        "-sn".into(),
        "-vf".into(),
        // Scaled before the thumbnail filter looks for a representative
        // frame, so it holds a hundred small frames rather than 4K ones.
        format!("{SQUARE},scale={width}:{height}:force_original_aspect_ratio=increase:flags=lanczos,thumbnail"),
        "-frames:v".into(),
        "1".into(),
    ]);
    args.extend(pam_out());
    let mut out = Vec::new();
    let ok = run(shared, &args, Duration::from_secs(30), |bytes| {
        out.extend_from_slice(bytes)
    });
    if !ok {
        return None;
    }
    parse_pam_stream(&out).into_iter().next()
}

fn peaks(shared: &Shared, job: &Job) -> Option<Peaks> {
    let mut args = quiet();
    args.extend([
        "-i".into(),
        job.path.to_string_lossy().into_owned(),
        "-vn".into(),
        "-sn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        PEAK_SAMPLE_RATE.to_string(),
        "-f".into(),
        "f32le".into(),
        "-c:a".into(),
        "pcm_f32le".into(),
        "pipe:1".into(),
    ]);
    let mut reducer = PeakReducer::new(PEAK_SAMPLE_RATE);
    let mut rest = Vec::new();
    let limit = Duration::from_secs_f64((30.0 + job.duration.max(0.0) * 0.25).min(900.0));
    let ok = run(shared, &args, limit, |bytes| {
        reducer.add_bytes(&mut rest, bytes)
    });
    ok.then(|| reducer.finish())
}

/// Makes a strip, or, for a long source, a frame of it: a long source is
/// sought a frame at a time, and between frames a poster or a sound asked
/// for meanwhile goes first (the job goes back to the head of the strips).
/// Some((job, strip)) once it is done, the strip None when none could be
/// had; None while it waits its turn again.
fn strip(shared: &Shared, mut job: Job) -> Option<(Job, Option<Filmstrip>)> {
    let duration = job.duration.max(0.0);
    let count = strip_frame_count(duration);
    let scale = format!("{SQUARE},scale=-2:{}:flags=lanczos", job.height);
    if duration <= DECODED_STRIP_SECONDS {
        // A short source is decoded once, its frames picked out evenly.
        let interval = if duration > 0.0 {
            duration / count as f64
        } else {
            1.0
        };
        let mut args = quiet();
        args.extend([
            "-i".into(),
            job.path.to_string_lossy().into_owned(),
            "-an".into(),
            "-sn".into(),
            "-vf".into(),
            format!("fps={},{scale}", 1.0 / interval),
            "-frames:v".into(),
            count.to_string(),
        ]);
        args.extend(pam_out());
        let mut out = Vec::new();
        let limit = Duration::from_secs_f64((30.0 + duration).min(900.0));
        let ok = run(shared, &args, limit, |bytes| out.extend_from_slice(bytes));
        let pictures = if ok {
            parse_pam_stream(&out)
        } else {
            Vec::new()
        };
        let strip = filmstrip(pictures.into_iter().map(Some).collect(), interval);
        return Some((job, strip));
    }
    let interval = duration / count as f64;
    while job.taken.len() < count {
        if shared.stop.load(Ordering::Relaxed) {
            return None;
        }
        let at = job.taken.len() as f64 * interval;
        let mut args = quiet();
        args.extend([
            "-ss".into(),
            format!("{at:.3}"),
            "-i".into(),
            job.path.to_string_lossy().into_owned(),
            "-an".into(),
            "-sn".into(),
            "-vf".into(),
            scale.clone(),
            "-frames:v".into(),
            "1".into(),
        ]);
        args.extend(pam_out());
        let mut out = Vec::new();
        let ok = run(shared, &args, Duration::from_secs(30), |bytes| {
            out.extend_from_slice(bytes)
        });
        let frame = ok
            .then(|| parse_pam_stream(&out).into_iter().next())
            .flatten();
        job.taken.push(frame);
        if job.taken.len() < count {
            let mut state = shared.lock();
            if !state.queues[0].is_empty() || !state.queues[1].is_empty() {
                state.queues[2].push_front(job);
                return None;
            }
        }
    }
    let taken = std::mem::take(&mut job.taken);
    Some((job, filmstrip(taken, interval)))
}

/// A strip of `frames`: one that could not be had (a seek past the last
/// keyframe, a read error) is the nearest one that could, so the spacing
/// holds. None when none could.
fn filmstrip(frames: Vec<Option<Picture>>, interval: f64) -> Option<Filmstrip> {
    let filled = fill_gaps(frames)?;
    let size = filled
        .first()
        .map(|picture| (picture.width, picture.height))?;
    let frames: Vec<Arc<RenderImage>> = filled.iter().filter_map(render_image).collect();
    (!frames.is_empty()).then_some(Filmstrip {
        frames,
        interval,
        size,
    })
}

/// `frames` with each one missing given the one before it, or at the
/// start the first that is there. None when none is.
pub fn fill_gaps<T: Clone>(frames: Vec<Option<T>>) -> Option<Vec<T>> {
    let first = frames.iter().flatten().next()?.clone();
    let mut last = first;
    Some(
        frames
            .into_iter()
            .map(|frame| {
                if let Some(frame) = frame {
                    last = frame;
                }
                last.clone()
            })
            .collect(),
    )
}

/// The FFmpeg the editor runs: the one kerfd runs
/// ([`kerf_bridge::ffmpeg_bin`]: `KERF_FFMPEG`, beside the editor, where
/// each package puts it, Homebrew's ffmpeg-full, then the PATH), found
/// once (off the window's thread, the first time a frame is read);
/// `ffmpeg` when none runs, to say so as it fails. Focus Region, the
/// artwork (posters, filmstrips, peaks) and a shot's graph's pictures read
/// with it.
pub fn ffmpeg_bin() -> &'static Path {
    Path::new(kerf_bridge::ffmpeg_bin())
}

/// Runs FFmpeg with `args`, handing what it writes to `sink` as it comes;
/// true when it ended well. It is stopped after `limit`, or when the
/// artwork is dropped.
fn run(shared: &Shared, args: &[String], limit: Duration, mut sink: impl FnMut(&[u8])) -> bool {
    // The FFmpeg kerfd would run (`KERF_FFMPEG`, beside the editor, the
    // PATH), found once off the window's thread.
    let mut command = Command::new(ffmpeg_bin());
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up for each picture.
        command.creation_flags(0x0800_0000);
    }
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };
    let child = Arc::new(Mutex::new(child));
    let (done, finished) = mpsc::channel::<()>();
    let watched = child.clone();
    let stopping = shared.stop.clone();
    let watchdog = std::thread::spawn(move || {
        let mut waited = Duration::ZERO;
        let tick = Duration::from_millis(200);
        loop {
            match finished.recv_timeout(tick) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    waited += tick;
                    if waited >= limit || stopping.load(Ordering::Relaxed) {
                        let _ = watched
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .kill();
                        return;
                    }
                }
            }
        }
    });
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match out.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => sink(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let _ = done.send(());
    let _ = watchdog.join();
    let status = child.lock().unwrap_or_else(PoisonError::into_inner).wait();
    status.is_ok_and(|status| status.success()) && !shared.stop.load(Ordering::Relaxed)
}
