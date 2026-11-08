//! The program's sound, mixed live as a render mixes it.
//!
//! Every clip heard runs the filters a render chains for it
//! ([`ProgramSound::cut`]) in a libavfilter graph of its own, and their
//! samples are summed, as the render's `amix` sums its chains with
//! `normalize=0`.

use crate::libav;
use crate::PlaybackError;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::context::Input;
use ffmpeg_next::util::format::Sample;
use ffmpeg_next::{decoder, filter, media, ChannelLayout, Rescale};
use kerf_core::program::{program_sound, LiveBus, ProgramSound, SoundCut, SAMPLE_RATE};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Catalog, Id, Project, Time, TrackKind};
use std::sync::Arc;

/// The mix is stereo, as a render's is.
pub(crate) const CHANNELS: usize = 2;
const FLICKS_PER_SAMPLE: i64 = FLICKS_PER_SECOND / SAMPLE_RATE as i64;
/// How much of a backward clip is decoded at once. A render decodes it
/// whole before playing any of it.
const BACKWARD_CHUNK: Time = Time::from_flicks(4 * FLICKS_PER_SECOND);
/// How far before a clip's source decoding starts, as FFmpeg's command line
/// seeks an input with reordered pictures (`3 * AV_TIME_BASE / 23`): the
/// first samples heard come out of a decoder that has warmed up.
const PREROLL_MICROS: i64 = 3_000_000 / 23;

/// The program sample at `time`, rounded as a render places sound.
fn sample_of(time: Time) -> i64 {
    kerf_core::program::samples(time)
}

fn time_of(sample: i64) -> Time {
    Time::from_flicks(sample * FLICKS_PER_SAMPLE)
}

/// The program's sound from some time on, stereo at [`SAMPLE_RATE`].
pub(crate) struct Mixer {
    sounds: Arc<[ProgramSound]>,
    /// The sounds still to start, by start, from `waiting[next]`.
    waiting: Vec<usize>,
    next: usize,
    voices: Vec<Voice>,
    /// The program's buses, each before the one it feeds.
    buses: Vec<BusStream>,
    /// The program sample the next frame mixed plays at.
    cursor: i64,
}

/// A bus of the program: what feeds it, summed, through its filters.
struct BusStream {
    /// `None` when its filters change nothing.
    graph: Option<filter::Graph>,
    to: Option<usize>,
    /// What has come out and not yet been mixed.
    pending: Vec<f32>,
}

impl BusStream {
    fn open(bus: &LiveBus) -> Self {
        let args = format!(
            "time_base=1/{SAMPLE_RATE}:sample_rate={SAMPLE_RATE}:sample_fmt=flt:channel_layout=0x{:x}",
            ChannelLayout::STEREO.bits()
        );
        // A bus that cannot play passes its sound on, rather than losing it.
        let graph = (!bus.filters.is_empty())
            .then(|| graph(&args, &bus.filters, SAMPLE_RATE, ChannelLayout::STEREO).ok())
            .flatten();
        Self {
            graph,
            to: bus.to,
            pending: Vec::new(),
        }
    }

    /// `input` through the filters, the same length: a filter that holds sound back has silence
    /// before it, as it has in a render.
    fn run(&mut self, input: &[f32], at: i64) -> Vec<f32> {
        let Some(graph) = &mut self.graph else {
            return input.to_vec();
        };
        let frames = input.len() / CHANNELS;
        let mut frame = ffmpeg::frame::Audio::new(
            Sample::F32(ffmpeg::format::sample::Type::Packed),
            frames,
            ChannelLayout::STEREO,
        );
        frame.set_rate(SAMPLE_RATE);
        frame.set_pts(Some(at));
        for (out, sample) in frame.data_mut(0).as_chunks_mut::<4>().0.iter_mut().zip(input) {
            out.copy_from_slice(&sample.to_ne_bytes());
        }
        let added = graph
            .get("in")
            .map(|mut source| source.source().add(&frame));
        if !matches!(added, Some(Ok(()))) {
            return input.to_vec();
        }
        if let Some(mut sink) = graph.get("out") {
            let mut out = ffmpeg::frame::Audio::empty();
            while sink.sink().frame(&mut out).is_ok() {
                if let Ok(samples) = libav::packed_f32(&out) {
                    self.pending.extend(samples);
                }
            }
        }
        let mut output = vec![0.0; input.len()];
        let have = self.pending.len().min(input.len());
        let at = input.len() - have;
        output[at..].copy_from_slice(&self.pending[..have]);
        self.pending.drain(..have);
        output
    }
}

impl Mixer {
    /// Mix from `at` seconds of the program on.
    pub fn new(sounds: Arc<[ProgramSound]>, at: f64) -> Self {
        let cursor = Time::from_seconds(at.max(0.0)).map_or(0, sample_of);
        let from = time_of(cursor);
        let mut waiting: Vec<usize> = (0..sounds.len())
            .filter(|index| sounds[*index].end > from)
            .collect();
        waiting.sort_by_key(|index| sounds[*index].start);
        let buses = sounds
            .first()
            .map(|sound| sound.buses.iter().map(BusStream::open).collect())
            .unwrap_or_default();
        Self {
            buses,
            sounds,
            waiting,
            next: 0,
            voices: Vec::new(),
            cursor,
        }
    }

    /// The next `frames` frames of the mix, interleaved.
    pub fn mix(&mut self, frames: usize) -> Vec<f32> {
        let end = self.cursor + frames as i64;
        while let Some(&index) = self.waiting.get(self.next) {
            let sound = &self.sounds[index];
            if sample_of(sound.start) >= end {
                break;
            }
            self.next += 1;
            let from = sound.start.max(time_of(self.cursor));
            if let Some(voice) = Voice::open(sound, index, from) {
                self.voices.push(voice);
            }
        }
        // One for each bus, then the mix.
        let mut mixes = vec![vec![0.0; frames * CHANNELS]; self.buses.len() + 1];
        let master = self.buses.len();
        let mut index = 0;
        while index < self.voices.len() {
            let target = self.sounds[self.voices[index].sound].bus.unwrap_or(master);
            let into = mixes.get_mut(target).expect("a bus of the program");
            if !self.voices[index].add(into, self.cursor) {
                index += 1;
                continue;
            }
            // A backward clip plays on in its next part.
            let voice = self.voices.swap_remove(index);
            let sound = &self.sounds[voice.sound];
            if voice.until < sound.end {
                if let Some(next) = Voice::open(sound, voice.sound, voice.until) {
                    self.voices.push(next);
                }
            }
        }
        for (place, bus) in self.buses.iter_mut().enumerate() {
            let out = bus.run(&mixes[place], self.cursor);
            let target = bus.to.unwrap_or(master);
            for (into, sample) in mixes[target].iter_mut().zip(&out) {
                *into += *sample;
            }
        }
        self.cursor = end;
        mixes.pop().unwrap_or_default()
    }

    /// The program time, in seconds, the next frame mixed plays at.
    pub fn position(&self) -> f64 {
        self.cursor as f64 / f64::from(SAMPLE_RATE)
    }
}

/// The sound of `project`'s program with only the audio tracks `solo` of
/// its program sequence heard, for monitoring: the others are muted in a
/// copy, so a solo mixes exactly as a render of those tracks would, nested
/// sequences and all, and the project itself never changes. A track muted
/// in the project stays muted. `None` when no id in `solo` names an audio
/// track of the program sequence, and the whole mix plays.
pub(crate) fn soloed_sounds(project: &Project, solo: &[Id]) -> Option<Vec<ProgramSound>> {
    let sequence = project.sequences.first()?;
    let heard = |kind: TrackKind, id: &Id| kind == TrackKind::Audio && solo.contains(id);
    if !sequence
        .tracks
        .iter()
        .any(|track| heard(track.kind, &track.id))
    {
        return None;
    }
    let mut soloed = project.clone();
    for track in &mut soloed.sequences[0].tracks {
        if track.kind == TrackKind::Audio && !heard(track.kind, &track.id) {
            track.muted = true;
        }
    }
    // A program a render cannot mix plays no sound, as it does unsoloed.
    Some(program_sound(&soloed, Catalog::builtin(), None).unwrap_or_default())
}

/// One clip's sound, decoded and run through its filters.
struct Voice {
    sound: usize,
    /// The program time this part of the sound ends at.
    until: Time,
    source: Source,
    /// Samples out of the filters, mixed up to `taken`.
    pending: Vec<f32>,
    taken: usize,
    /// The program sample `pending[taken..]` starts at.
    next: i64,
}

impl Voice {
    /// The sound of `sound` from program time `from` on. A clip that cannot
    /// play stays silent, as it does when its media is missing.
    fn open(sound: &ProgramSound, index: usize, from: Time) -> Option<Self> {
        let until = if sound.reverse() {
            (from + BACKWARD_CHUNK).min(sound.end)
        } else {
            sound.end
        };
        let cut = sound.cut(Catalog::builtin(), from, until).ok()??;
        let source = Source::open(&sound.path, &cut).ok()?;
        Some(Self {
            sound: index,
            until,
            source,
            pending: Vec::new(),
            taken: 0,
            next: sample_of(cut.start),
        })
    }

    /// Add this voice's samples to `mixed`, whose first frame plays at
    /// program sample `cursor`; whether the voice has ended.
    fn add(&mut self, mixed: &mut [f32], cursor: i64) -> bool {
        let end = cursor + (mixed.len() / CHANNELS) as i64;
        while self.next < end {
            if self.taken == self.pending.len() {
                match self.source.next_samples() {
                    Ok(Some(samples)) => {
                        self.pending = samples;
                        self.taken = 0;
                    }
                    Ok(None) | Err(_) => return true,
                }
                continue;
            }
            let available = (self.pending.len() - self.taken) / CHANNELS;
            let count = if self.next < cursor {
                available.min((cursor - self.next) as usize)
            } else {
                let at = (self.next - cursor) as usize * CHANNELS;
                let count = available.min((end - self.next) as usize);
                let samples = &self.pending[self.taken..self.taken + count * CHANNELS];
                for (out, sample) in mixed[at..at + count * CHANNELS].iter_mut().zip(samples) {
                    *out += *sample;
                }
                count
            };
            self.taken += count * CHANNELS;
            self.next += count as i64;
        }
        false
    }
}

/// A media file's sound, decoded from a cut's source and run through its
/// filters.
struct Source {
    input: Input,
    decoder: decoder::Audio,
    graph: filter::Graph,
    stream: usize,
    time_base: ffmpeg::Rational,
    rate: i32,
    /// Where the filters' time starts, in the stream's time base.
    zero: i64,
    /// The samples from `zero` the filters can use.
    span: i64,
    /// Where the next frame starts, in samples from `zero`, for a frame
    /// without a timestamp.
    expected: Option<i64>,
    /// No more packets are read; `flushed` once the filters know it.
    eof: bool,
    flushed: bool,
}

impl Source {
    fn open(path: &str, cut: &SoundCut) -> Result<Self, PlaybackError> {
        libav::ensure_init().map_err(audio_error)?;
        let mut input = ffmpeg::format::input(&path).map_err(ffmpeg_error)?;
        let (stream, time_base, parameters) = {
            let stream = input
                .streams()
                .best(media::Type::Audio)
                .ok_or_else(|| PlaybackError::Audio(format!("{path}: no audio stream")))?;
            (stream.index(), stream.time_base(), stream.parameters())
        };
        // Timed as FFmpeg's command line times it, which also moves the
        // timestamps past the samples a codec primes with.
        let decoder = ffmpeg::codec::context::Context::from_parameters(parameters)
            .and_then(|context| {
                let mut decoder = context.decoder();
                decoder.set_packet_time_base(time_base);
                decoder.audio()
            })
            .map_err(ffmpeg_error)?;
        let rate = decoder.rate().max(1) as i32;
        // As FFmpeg's `-ss` reads it: the source time past the file's own
        // start, in microseconds.
        // SAFETY: `input` owns a valid, opened format context.
        let first = unsafe { (*input.as_ptr()).start_time };
        let start = micros(cut.source.0)
            + if first == ffmpeg::ffi::AV_NOPTS_VALUE {
                0
            } else {
                first
            };
        if cut.source.0.is_positive() {
            // A seek that fails reads from the file's start, which the
            // filters trim just the same.
            let at = start - PREROLL_MICROS;
            let _ = input.seek(at, ..at);
        }
        let span = ((cut.source.1 - cut.source.0).seconds() * f64::from(rate)).ceil() as i64;
        let graph = clip_graph(&decoder, rate, &cut.filters)?;
        Ok(Self {
            input,
            decoder,
            graph,
            stream,
            time_base,
            rate,
            zero: start.rescale(ffmpeg::rescale::TIME_BASE, time_base),
            span,
            expected: None,
            eof: false,
            flushed: false,
        })
    }

    /// The next samples out of the filters, stereo and interleaved; `None`
    /// once they end.
    fn next_samples(&mut self) -> Result<Option<Vec<f32>>, PlaybackError> {
        loop {
            let mut filtered = ffmpeg::frame::Audio::empty();
            let pulled = self
                .graph
                .get("out")
                .ok_or_else(|| PlaybackError::Audio("missing abuffersink".into()))?
                .sink()
                .frame(&mut filtered);
            match pulled {
                Ok(()) => {
                    let samples = libav::packed_f32(&filtered).map_err(audio_error)?;
                    if !samples.is_empty() {
                        return Ok(Some(samples));
                    }
                }
                Err(error) if libav::is_eof(&error) => return Ok(None),
                Err(error) if libav::is_again(&error) => {
                    if self.flushed {
                        return Ok(None);
                    }
                    self.pump()?;
                }
                Err(error) => return Err(ffmpeg_error(error)),
            }
        }
    }

    fn pump(&mut self) -> Result<(), PlaybackError> {
        if self.eof {
            self.flushed = true;
            if let Some(mut source) = self.graph.get("in") {
                let _ = source.source().flush();
            }
            return Ok(());
        }
        let packet = self
            .input
            .packets()
            .next()
            .map(|(stream, packet)| (stream.index(), packet));
        match packet {
            Some((index, packet)) if index == self.stream => {
                loop {
                    match self.decoder.send_packet(&packet) {
                        Ok(()) => break,
                        Err(error) if libav::is_again(&error) => self.drain()?,
                        Err(error) if libav::is_eof(&error) => {
                            self.eof = true;
                            break;
                        }
                        Err(error) => return Err(ffmpeg_error(error)),
                    }
                }
                self.drain()
            }
            Some(_) => Ok(()),
            None => {
                self.eof = true;
                let _ = self.decoder.send_eof();
                self.drain()
            }
        }
    }

    /// Hand the decoded frames to the filters, timed from the source's
    /// start in samples, as FFmpeg's command line times them.
    fn drain(&mut self) -> Result<(), PlaybackError> {
        let mut decoded = ffmpeg::frame::Audio::empty();
        loop {
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => {}
                Err(error) if libav::is_again(&error) || libav::is_eof(&error) => return Ok(()),
                Err(error) => return Err(ffmpeg_error(error)),
            }
            let Some(at) = decoded
                .timestamp()
                .map(|at| (at - self.zero).rescale(self.time_base, (1, self.rate)))
                .or(self.expected)
            else {
                continue;
            };
            self.expected = Some(at + decoded.samples() as i64);
            if at >= self.span {
                // Past what the filters keep.
                self.eof = true;
                continue;
            }
            decoded.set_pts(Some(at));
            self.graph
                .get("in")
                .ok_or_else(|| PlaybackError::Audio("missing abuffer source".into()))?
                .source()
                .add(&decoded)
                .map_err(ffmpeg_error)?;
        }
    }
}

/// The graph that runs a clip's `chain` on what `decoder` decodes, into
/// packed stereo floats at [`SAMPLE_RATE`].
fn clip_graph(
    decoder: &decoder::Audio,
    rate: i32,
    chain: &str,
) -> Result<filter::Graph, PlaybackError> {
    let layout = decoder.channel_layout();
    let layout = if layout.is_empty() {
        ChannelLayout::default(i32::from(decoder.channels()))
    } else {
        layout
    };
    let args = format!(
        "time_base=1/{rate}:sample_rate={rate}:sample_fmt={}:channel_layout=0x{:x}",
        decoder.format().name(),
        layout.bits()
    );
    graph(&args, chain, SAMPLE_RATE, ChannelLayout::STEREO)
}

/// `chain` from an `abuffer` taking `args`, into packed floats at `rate`
/// in `layout`.
fn graph(
    args: &str,
    chain: &str,
    rate: u32,
    layout: ChannelLayout,
) -> Result<filter::Graph, PlaybackError> {
    let mut graph = filter::Graph::new();
    let buffer = filter::find("abuffer")
        .ok_or_else(|| PlaybackError::Audio("libavfilter abuffer is unavailable".into()))?;
    let sink = filter::find("abuffersink")
        .ok_or_else(|| PlaybackError::Audio("libavfilter abuffersink is unavailable".into()))?;
    graph.add(&buffer, "in", args).map_err(ffmpeg_error)?;
    graph.add(&sink, "out", "").map_err(ffmpeg_error)?;
    let chain = format!(
        "{chain},aformat=sample_fmts=flt:sample_rates={rate}:channel_layouts=0x{:x}",
        layout.bits()
    );
    graph
        .output("in", 0)
        .map_err(ffmpeg_error)?
        .input("out", 0)
        .map_err(ffmpeg_error)?
        .parse(&chain)
        .map_err(ffmpeg_error)?;
    graph.validate().map_err(ffmpeg_error)?;
    Ok(graph)
}

/// The mix resampled for an output device that takes another rate or
/// other channels.
pub(crate) struct Resampler {
    graph: filter::Graph,
    pts: i64,
}

impl Resampler {
    /// `None` when the device takes the mix as it is.
    pub fn new(rate: u32, channels: usize) -> Result<Option<Self>, PlaybackError> {
        if rate == SAMPLE_RATE && channels == CHANNELS {
            return Ok(None);
        }
        let args = format!(
            "time_base=1/{SAMPLE_RATE}:sample_rate={SAMPLE_RATE}:sample_fmt=flt:channel_layout=stereo"
        );
        let layout = ChannelLayout::default(channels.max(1) as i32);
        let graph = graph(&args, &format!("aresample={}", rate.max(1)), rate, layout)?;
        Ok(Some(Self { graph, pts: 0 }))
    }

    /// The device's samples for `mixed`, interleaved; the resampler keeps
    /// back the few it needs to filter on.
    pub fn convert(&mut self, mixed: &[f32]) -> Result<Vec<f32>, PlaybackError> {
        let frames = mixed.len() / CHANNELS;
        let mut frame = ffmpeg::frame::Audio::new(
            Sample::F32(ffmpeg::format::sample::Type::Packed),
            frames,
            ChannelLayout::STEREO,
        );
        frame.set_rate(SAMPLE_RATE);
        frame.set_pts(Some(self.pts));
        self.pts += frames as i64;
        for (bytes, sample) in frame.data_mut(0).as_chunks_mut::<4>().0.iter_mut().zip(mixed) {
            bytes.copy_from_slice(&sample.to_ne_bytes());
        }
        self.graph
            .get("in")
            .ok_or_else(|| PlaybackError::Audio("missing abuffer source".into()))?
            .source()
            .add(&frame)
            .map_err(ffmpeg_error)?;
        let mut converted = Vec::new();
        loop {
            let mut out = ffmpeg::frame::Audio::empty();
            let pulled = self
                .graph
                .get("out")
                .ok_or_else(|| PlaybackError::Audio("missing abuffersink".into()))?
                .sink()
                .frame(&mut out);
            match pulled {
                Ok(()) => converted.extend(libav::packed_f32(&out).map_err(audio_error)?),
                Err(error) if libav::is_again(&error) || libav::is_eof(&error) => {
                    return Ok(converted)
                }
                Err(error) => return Err(ffmpeg_error(error)),
            }
        }
    }
}

/// Microseconds in `time`, as FFmpeg reads a time printed with nine
/// decimals.
fn micros(time: Time) -> i64 {
    let flicks = i128::from(time.flicks());
    let per = i128::from(FLICKS_PER_SECOND);
    let nanos = (flicks.abs() * 1_000_000_000 + per / 2) / per;
    (nanos / 1000 * flicks.signum()) as i64
}

fn ffmpeg_error(error: ffmpeg::Error) -> PlaybackError {
    audio_error(libav::map_error(error))
}

fn audio_error(error: PlaybackError) -> PlaybackError {
    match error {
        PlaybackError::Ffmpeg(message) => PlaybackError::Audio(message),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{Mixer, CHANNELS};
    use crate::timeline::PlaybackTimeline;
    use kerf_core::program::{compile_program, ProgramOptions};
    use kerf_model::{Catalog, Project, Time};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn ffmpeg() -> Option<PathBuf> {
        let binary = std::env::var_os("KERF_FFMPEG").map_or_else(|| "ffmpeg".into(), PathBuf::from);
        let output = Command::new(&binary).arg("-version").output().ok()?;
        output.status.success().then_some(binary)
    }

    fn run(ffmpeg: &Path, args: &[&str]) {
        let output = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .output()
            .expect("run ffmpeg");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The sound a render of `range` mixes.
    fn rendered(
        ffmpeg: &Path,
        project: &Project,
        range: Option<(Time, Time)>,
        out: &Path,
    ) -> Vec<f32> {
        let options = ProgramOptions {
            range,
            audio_only: true,
            ..ProgramOptions::default()
        };
        let compiled = compile_program(project, Catalog::builtin(), &options).expect("compiles");
        let mut args: Vec<&str> = compiled.argv[1..].iter().map(String::as_str).collect();
        let out = out.to_str().expect("UTF-8 path");
        args.extend(["-c:a", "pcm_f32le", "-f", "f32le", out]);
        run(ffmpeg, &args);
        std::fs::read(out)
            .expect("rendered sound")
            .as_chunks::<4>().0.iter()
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect()
    }

    /// The sound playback mixes from `at` seconds on.
    fn live(project: &Project, at: f64, samples: usize) -> Vec<f32> {
        let timeline = PlaybackTimeline::from_project(project).expect("timeline");
        let mut mixer = Mixer::new(timeline.sounds(), at);
        let mut mixed = Vec::new();
        while mixed.len() < samples {
            mixed.extend(mixer.mix(1024));
        }
        mixed.truncate(samples);
        mixed
    }

    fn rms(samples: impl Iterator<Item = f32>) -> f64 {
        let (sum, count) = samples.fold((0.0, 0), |(sum, count), sample| {
            (sum + f64::from(sample).powi(2), count + 1)
        });
        (sum / f64::from(count.max(1))).sqrt()
    }

    /// How far `live` is from `rendered`, against how loud the render is.
    fn difference(rendered: &[f32], live: &[f32]) -> f64 {
        let error = rms(rendered.iter().zip(live).map(|(a, b)| a - b));
        error / rms(rendered.iter().copied())
    }

    /// Dialog, a tone in AAC beside a picture, and music, pink noise at
    /// 44.1 kHz.
    fn fixtures(ffmpeg: &Path, directory: &Path) -> (String, String) {
        let encode = |name: &str, args: &str| {
            let path = directory
                .join(name)
                .to_str()
                .expect("UTF-8 path")
                .to_owned();
            run(
                ffmpeg,
                &[args.split(' ').collect(), vec![path.as_str()]].concat(),
            );
            path
        };
        let dialog = encode(
            "dialog.mp4",
            "-f lavfi -i testsrc2=size=64x36:rate=30 -f lavfi -i sine=frequency=440:sample_rate=48000 \
             -t 4 -c:v libx264 -pix_fmt yuv420p -c:a aac -ac 2",
        );
        let music = encode(
            "music.wav",
            "-f lavfi -i anoisesrc=color=pink:sample_rate=44100:seed=7:amplitude=0.3 \
             -t 4 -ac 2 -c:a pcm_s16le",
        );
        (dialog, music)
    }

    /// The media clips heard when `solo` is soloed.
    fn heard(project: &Project, solo: &[&str]) -> Vec<String> {
        let timeline = PlaybackTimeline::from_project(project).expect("timeline");
        let solo: Vec<kerf_model::Id> = solo
            .iter()
            .map(|id| kerf_model::Id::new(*id).expect("id"))
            .collect();
        let mut clips: Vec<String> = timeline
            .soloed_sounds(&solo)
            .iter()
            .map(|sound| sound.clip.to_string())
            .collect();
        clips.sort();
        clips
    }

    #[test]
    fn a_solo_mixes_only_the_tracks_soloed() {
        let text = "kerf 1\nproject name=\"Solo\"\n\
             media d path=\"/tmp/solo-d.wav\" rate=30 dur=4s audio=2ch@48000\n\
             sequence s1 rate=30 size=64x36 audio=48000\n\
             \x20 track V1 video\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=d src=0s at=0s dur=1s\n\
             \x20 track A2 audio\n\
             \x20   clip b1 media=d src=0s at=0s dur=1s\n\
             \x20   clip n1 seq=inner src=0s at=1s dur=1s\n\
             \x20 track A3 audio muted=true\n\
             \x20   clip c1 media=d src=0s at=0s dur=1s\n\
             sequence inner rate=30 size=64x36 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip i1 media=d src=2s at=0s dur=1s\n";
        let project = kerf_lang::parse_document(text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let before = project.clone();
        assert_eq!(heard(&project, &[]), ["a1", "b1", "i1"]);
        assert_eq!(heard(&project, &["A1"]), ["a1"]);
        // A soloed track brings the sequences nested on it.
        assert_eq!(heard(&project, &["A2"]), ["b1", "i1"]);
        assert_eq!(heard(&project, &["A1", "A2"]), ["a1", "b1", "i1"]);
        // A muted track stays muted, soloed or not.
        assert!(heard(&project, &["A3"]).is_empty());
        // Ids that name no audio track of the program leave the mix whole:
        // a video track, a track of a nested sequence, a deleted one.
        assert_eq!(heard(&project, &["V1"]), ["a1", "b1", "i1"]);
        assert_eq!(heard(&project, &["gone"]), ["a1", "b1", "i1"]);
        assert_eq!(project, before, "a solo never changes the project");
    }

    /// Where each burst of tone starts in `samples`, in seconds.
    fn onsets(samples: &[f32]) -> Vec<f64> {
        let block = 96;
        let loud: Vec<bool> = samples
            .chunks(block * CHANNELS)
            .map(|chunk| chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32 > 0.01)
            .collect();
        let quiet = 9_600 / block;
        loud.iter()
            .enumerate()
            .filter(|(at, on)| **on && !loud[at.saturating_sub(quiet)..*at].contains(&true))
            .map(|(at, _)| (at * block) as f64 / 48_000.0)
            .collect()
    }

    #[test]
    fn the_live_mix_hears_buses_as_a_render_does() {
        let Some(ffmpeg) = ffmpeg() else {
            eprintln!("skipping the bus check: ffmpeg is unavailable");
            return;
        };
        let directory = tempfile::tempdir().expect("temporary directory");
        let (dialog, music) = fixtures(&ffmpeg, directory.path());
        // Dialog and music through a compressor that must hear them together, a gain that moves, and
        // a second bus before the mix.
        let text = format!(
            "kerf 1\nproject name=\"Mix\"\n\
             media d path=\"{dialog}\" rate=30 dur=4s video=64x36 audio=2ch@48000\n\
             media m path=\"{music}\" rate=30 dur=4s audio=2ch@44100\n\
             sequence s1 rate=30 size=64x36 audio=48000\n\
             \x20 bus all to=out gain={{0f:0 60f:-9}}\n\
             \x20   fx audio.compressor@1 threshold_db=-24 ratio=6 attack_ms=5 release_ms=80\n\
             \x20 bus out gain=-2 pan=0.2\n\
             \x20 track A1 audio bus=all\n\
             \x20   clip a1 media=d src=0.5s at=0s dur=3s\n\
             \x20 track A2 audio bus=all\n\
             \x20   clip m1 media=m src=0s at=0.5s dur=2s\n\
             \x20 track A3 audio\n\
             \x20   clip o1 media=d src=0s at=1s dur=1s\n\
             \x20     set gain=-12\n"
        );
        let project = kerf_lang::parse_document(&text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let out = directory.path().join("render.f32");
        let whole = rendered(&ffmpeg, &project, None, &out);
        assert!(!whole.is_empty());
        let played = live(&project, 0.0, whole.len());
        let off = difference(&whole, &played);
        eprintln!("live against render through buses: {off:e}");
        assert!(off < 0.03, "{off}");
        // Without its compressor it is another mix, so the check can tell.
        let bare = kerf_lang::parse_document(
            &text.replace("\x20   fx audio.compressor@1 threshold_db=-24 ratio=6 attack_ms=5 release_ms=80\n", "").replace("    fx audio.compressor@1 threshold_db=-24 ratio=6 attack_ms=5 release_ms=80\n", ""),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project;
        let without = live(&bare, 0.0, whole.len());
        assert!(difference(&whole, &without) > 0.1);
    }

    #[test]
    fn a_ramp_is_heard_live_and_in_a_render_where_the_ramp_says() {
        let Some(ffmpeg) = ffmpeg() else {
            eprintln!("skipping the ramp check: ffmpeg is unavailable");
            return;
        };
        let directory = tempfile::tempdir().expect("temporary directory");
        // A tenth of a second of tone every half second. Not 1 kHz, whose samples repeat every 192
        // bytes: libav then takes the file for an MPEG transport stream.
        let path = directory.path().join("bursts.wav");
        let path = path.to_str().expect("UTF-8 path");
        run(
            &ffmpeg,
            &[
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=if(lt(mod(t\\,0.5)\\,0.1)\\,0.8*sin(2*PI*997*t)\\,0):s=48000:d=8",
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
                path,
            ],
        );
        // One speed for a second, then down to half in a line over two, then half.
        let text = format!(
            "kerf 1\nproject name=\"Ramp\"\n\
             media d path=\"{path}\" rate=30 dur=8s audio=2ch@48000\n\
             sequence s1 rate=30 size=64x36 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=d src=0s at=0s dur=6s speed={{0f:1 30f:1 90f:1/2}}\n"
        );
        let project = kerf_lang::parse_document(&text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let clip = project.clip("a1").expect("the clip");
        let expected = |source: f64| {
            clip.played_at(Time::from_seconds(source).expect("a time"))
                .expect("a source")
                .seconds()
        };
        let played = live(&project, 0.0, 6 * 48_000 * CHANNELS);
        let heard = onsets(&played);
        assert!(heard.len() >= 8, "{heard:?}");
        for (index, onset) in heard.iter().enumerate() {
            let wanted = expected(index as f64 * 0.5);
            assert!(
                (onset - wanted).abs() < 0.06,
                "burst {index} heard at {onset}, the ramp says {wanted}"
            );
        }
        // A render reads what playback reads.
        let baked = kerf_compose::ramps::with_sound(&project, |_, _, _| None);
        let out = directory.path().join("render.f32");
        let whole = rendered(&ffmpeg, &baked, None, &out);
        assert!(difference(&whole, &played) < 1e-2);
        // Seeking lands in the same sound.
        let resumed = live(&project, 2.0, 2 * 48_000 * CHANNELS);
        let start = 2 * 48_000 * CHANNELS;
        assert!(difference(&played[start..start + resumed.len()], &resumed) < 1e-2);
    }

    #[test]
    fn a_reversed_ramp_plays_its_sound_backward() {
        let Some(ffmpeg) = ffmpeg() else {
            eprintln!("skipping the ramp check: ffmpeg is unavailable");
            return;
        };
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("burst.wav");
        let path = path.to_str().expect("UTF-8 path");
        // One burst, at a second and a half of three.
        run(
            &ffmpeg,
            &[
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=if(between(t\\,1.5\\,1.6)\\,0.8*sin(2*PI*1000*t)\\,0):s=48000:d=3",
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
                path,
            ],
        );
        let text = format!(
            "kerf 1\nproject name=\"Ramp\"\n\
             media d path=\"{path}\" rate=30 dur=3s audio=2ch@48000\n\
             sequence s1 rate=30 size=64x36 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=d src=0s at=0s dur=2s speed={{0f:-2 30f:-1}}\n"
        );
        let project = kerf_lang::parse_document(&text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let clip = project.clip("a1").expect("the clip");
        // The end of the burst comes first when played backward.
        let wanted = clip
            .played_at(Time::from_seconds(1.6).expect("a time"))
            .expect("a source")
            .seconds();
        let played = live(&project, 0.0, 2 * 48_000 * CHANNELS);
        let heard = onsets(&played);
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert!(
            (heard[0] - wanted).abs() < 0.06,
            "{} against {wanted}",
            heard[0]
        );
    }

    #[test]
    fn the_live_mix_sounds_like_a_render() {
        let Some(ffmpeg) = ffmpeg() else {
            eprintln!("skipping the live mix check: ffmpeg is unavailable");
            return;
        };
        let directory = tempfile::tempdir().expect("temporary directory");
        let (dialog, music) = fixtures(&ffmpeg, directory.path());
        // Dialog crossfades on A1 under music on A2; A3 plays a clip
        // backward and a nested sequence of both.
        let text = format!(
            "kerf 1\nproject name=\"Mix\"\n\
             media d path=\"{dialog}\" rate=30 dur=4s video=64x36 audio=2ch@48000\n\
             media m path=\"{music}\" rate=30 dur=4s audio=2ch@44100\n\
             sequence s1 rate=30 size=64x36 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=d src=0.5s at=0s dur=1s\n\
             \x20   clip a2 media=d src=2s at=1s dur=1s\n\
             \x20     set gain={{0f:-12 15f:0:ease}}\n\
             \x20   transition x1 from=a1 to=a2 fx=crossfade@1 dur=10f\n\
             \x20 track A2 audio\n\
             \x20   clip m1 media=m src=0s at=0.5s dur=2s\n\
             \x20     set gain=-6 pan=-0.3\n\
             \x20     fx audio.eq3 low=4 high=-3\n\
             \x20 track A3 audio\n\
             \x20   clip r1 media=d src=1s at=1.5s dur=1s speed=-1\n\
             \x20   clip n1 seq=inner src=0s at=2.5s dur=1s\n\
             \x20     set gain=-3\n\
             sequence inner rate=30 size=64x36 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip i1 media=m src=1s at=0s dur=1s\n\
             \x20 track A2 audio\n\
             \x20   clip i2 media=d src=0s at=0.5s dur=1s\n"
        );
        let project = kerf_lang::parse_document(&text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let out = directory.path().join("render.f32");
        let whole = rendered(&ffmpeg, &project, None, &out);
        assert_eq!(whole.len(), 168_000 * CHANNELS);
        let played = live(&project, 0.0, whole.len());
        let from_start = difference(&whole, &played);
        // From inside the crossfade, against a render of the rest.
        let at = Time::from_seconds(1.1).unwrap();
        let rest = rendered(
            &ffmpeg,
            &project,
            Some((at, Time::from_seconds(3.5).unwrap())),
            &out,
        );
        let resumed = live(&project, 1.1, rest.len());
        let from_seek = difference(&rest, &resumed);
        eprintln!(
            "live mix against a render: {from_start:e} from the start, {from_seek:e} from a seek"
        );
        assert!(from_start < 1e-3, "{from_start}");
        assert!(from_seek < 1e-3, "{from_seek}");
    }
}
