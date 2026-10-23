use crate::libav;
use crate::timeline::{PlaybackClip, PlaybackSource, PlaybackTimeline};
use crate::{ClockSource, PlaybackError};
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::context::Input;
use ffmpeg_next::util::format::Sample;
use ffmpeg_next::{decoder, filter, media, ChannelLayout};
#[cfg(feature = "native-audio")]
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const AUDIO_COMMAND_QUEUE_CAPACITY: usize = 16;
const AUDIO_BUFFER_SECONDS: f64 = 0.35;
const AUDIO_TICK: Duration = Duration::from_millis(4);

#[derive(Clone)]
pub(crate) struct AudioRuntime {
    commands: SyncSender<AudioCommand>,
    clock: AudioClock,
}

impl AudioRuntime {
    pub fn spawn(timeline: PlaybackTimeline, enabled: bool) -> Result<Self, PlaybackError> {
        let (commands, receiver) = mpsc::sync_channel(AUDIO_COMMAND_QUEUE_CAPACITY);
        let clock = AudioClock::new();
        let worker_clock = clock.clone();
        thread::Builder::new()
            .name("kerf-audio".into())
            .spawn(move || {
                let output = make_audio_output(worker_clock.clone(), enabled);
                AudioWorker {
                    timeline,
                    commands: receiver,
                    clock: worker_clock,
                    output,
                    decoder: None,
                    playing: false,
                    position: 0.0,
                    volume: 1.0,
                    muted: false,
                    enabled,
                }
                .run();
            })
            .map_err(|error| {
                PlaybackError::Audio(format!("failed to start audio worker: {error}"))
            })?;
        Ok(Self { commands, clock })
    }

    pub fn open(&self, timeline: PlaybackTimeline) {
        self.clock.set_position(0.0);
        self.clock.set_playing(false);
        self.send(AudioCommand::Open(Box::new(timeline)));
    }

    pub fn play(&self, position: f64) {
        self.clock.play_at(position);
        self.send(AudioCommand::Play(position));
    }

    pub fn pause(&self) {
        self.clock.pause();
        self.send(AudioCommand::Pause);
    }

    pub fn seek(&self, position: f64) {
        self.clock.set_position(position);
        self.send(AudioCommand::Seek(position));
    }

    pub fn set_volume(&self, volume: f32) {
        self.send(AudioCommand::SetVolume(volume));
    }

    pub fn set_muted(&self, muted: bool) {
        self.send(AudioCommand::SetMuted(muted));
    }

    pub fn shutdown(&self) {
        self.send(AudioCommand::Shutdown);
    }

    pub fn clock(&self, duration: f64) -> crate::PlaybackClock {
        crate::PlaybackClock {
            position: self.clock.now().clamp(0.0, duration.max(0.0)),
            duration,
            source: self.clock.source(),
        }
    }

    fn send(&self, command: AudioCommand) {
        let _ = self.commands.send(command);
    }
}

#[derive(Clone)]
pub(crate) struct AudioClock {
    inner: Arc<AudioClockState>,
}

struct AudioClockState {
    played_frames: AtomicU64,
    base_frame: AtomicU64,
    base_position: AtomicU64,
    sample_rate: AtomicU32,
    playing: AtomicBool,
    source: AtomicU8,
    anchor: Mutex<ClockAnchor>,
}

struct ClockAnchor {
    position: f64,
    started: Option<Instant>,
}

impl AudioClock {
    fn new() -> Self {
        Self {
            inner: Arc::new(AudioClockState {
                played_frames: AtomicU64::new(0),
                base_frame: AtomicU64::new(0),
                base_position: AtomicU64::new(0.0_f64.to_bits()),
                sample_rate: AtomicU32::new(48_000),
                playing: AtomicBool::new(false),
                source: AtomicU8::new(ClockSource::MonotonicFallback as u8),
                anchor: Mutex::new(ClockAnchor {
                    position: 0.0,
                    started: None,
                }),
            }),
        }
    }

    #[cfg(feature = "native-audio")]
    fn set_device_clock(&self, sample_rate: u32) {
        let position = self.now();
        self.inner
            .sample_rate
            .store(sample_rate.max(1), Ordering::Release);
        self.inner.base_frame.store(
            self.inner.played_frames.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.inner
            .base_position
            .store(position.to_bits(), Ordering::Release);
        self.inner
            .source
            .store(ClockSource::AudioDevice as u8, Ordering::Release);
    }

    fn set_position(&self, position: f64) {
        let position = position.max(0.0);
        self.inner.base_frame.store(
            self.inner.played_frames.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.inner
            .base_position
            .store(position.to_bits(), Ordering::Release);
        let mut anchor = self
            .inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        anchor.position = position;
        anchor.started = self
            .inner
            .playing
            .load(Ordering::Acquire)
            .then(Instant::now);
    }

    fn play_at(&self, position: f64) {
        self.set_position(position);
        self.inner.playing.store(true, Ordering::Release);
        let mut anchor = self
            .inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        anchor.position = position.max(0.0);
        anchor.started = Some(Instant::now());
    }

    fn pause(&self) {
        let position = self.now();
        self.inner.playing.store(false, Ordering::Release);
        self.set_position(position);
    }

    fn set_playing(&self, playing: bool) {
        if playing {
            self.inner.playing.store(true, Ordering::Release);
            let mut anchor = self
                .inner
                .anchor
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            anchor.started = Some(Instant::now());
        } else {
            self.pause();
        }
    }

    fn is_playing(&self) -> bool {
        self.inner.playing.load(Ordering::Acquire)
    }

    #[cfg(feature = "native-audio")]
    fn record_frames(&self, frames: u64) {
        self.inner
            .played_frames
            .fetch_add(frames, Ordering::Release);
    }

    fn now(&self) -> f64 {
        if self.source() == ClockSource::AudioDevice {
            let played = self.inner.played_frames.load(Ordering::Acquire);
            let base = self.inner.base_frame.load(Ordering::Acquire);
            let position = f64::from_bits(self.inner.base_position.load(Ordering::Acquire));
            let rate = self.inner.sample_rate.load(Ordering::Acquire).max(1) as f64;
            return position + played.saturating_sub(base) as f64 / rate;
        }
        let anchor = self
            .inner
            .anchor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match anchor.started {
            Some(started) if self.is_playing() => anchor.position + started.elapsed().as_secs_f64(),
            _ => anchor.position,
        }
    }

    fn source(&self) -> ClockSource {
        if self.inner.source.load(Ordering::Acquire) == ClockSource::AudioDevice as u8 {
            ClockSource::AudioDevice
        } else {
            ClockSource::MonotonicFallback
        }
    }
}

enum AudioCommand {
    Open(Box<PlaybackTimeline>),
    Play(f64),
    Pause,
    Seek(f64),
    SetVolume(f32),
    SetMuted(bool),
    Shutdown,
}

trait AudioOutput {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> usize;
    fn push(&self, samples: &[f32]);
    fn queued_frames(&self) -> usize;
    fn clear(&self);
    fn set_volume(&self, volume: f32);
    fn set_muted(&self, muted: bool);
}

struct AudioWorker {
    timeline: PlaybackTimeline,
    commands: Receiver<AudioCommand>,
    clock: AudioClock,
    output: Box<dyn AudioOutput>,
    decoder: Option<AudioDecoder>,
    playing: bool,
    position: f64,
    volume: f32,
    muted: bool,
    enabled: bool,
}

impl AudioWorker {
    fn run(mut self) {
        loop {
            let result = if self.playing {
                self.commands.try_recv().map_err(|error| match error {
                    TryRecvError::Empty => RecvTimeoutError::Timeout,
                    TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
                })
            } else {
                self.commands.recv_timeout(Duration::from_millis(100))
            };
            match result {
                Ok(command) => {
                    if !self.handle_command(command) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.playing {
                self.decode_until_buffered();
                thread::sleep(AUDIO_TICK);
            }
        }
        self.stop_decoder();
    }

    fn handle_command(&mut self, command: AudioCommand) -> bool {
        match command {
            AudioCommand::Open(timeline) => {
                self.stop_decoder();
                self.timeline = *timeline;
                self.position = 0.0;
                self.playing = false;
                self.output.clear();
                self.clock.set_position(0.0);
                self.clock.set_playing(false);
            }
            AudioCommand::Play(position) => {
                self.stop_decoder();
                self.position = position;
                self.playing = true;
                self.output.clear();
                self.output.set_volume(self.volume);
                self.output.set_muted(self.muted);
                self.clock.play_at(position);
            }
            AudioCommand::Pause => {
                self.position = self.clock.now();
                self.playing = false;
                self.stop_decoder();
                self.output.clear();
                self.clock.pause();
            }
            AudioCommand::Seek(position) => {
                self.position = position;
                self.stop_decoder();
                self.output.clear();
                self.clock.set_position(position);
            }
            AudioCommand::SetVolume(volume) => {
                self.volume = volume;
                self.output.set_volume(volume);
            }
            AudioCommand::SetMuted(muted) => {
                self.muted = muted;
                self.output.set_muted(muted);
            }
            AudioCommand::Shutdown => return false,
        }
        true
    }

    fn decode_until_buffered(&mut self) {
        if !self.enabled
            || self.output.queued_frames()
                >= (self.output.sample_rate() as f64 * AUDIO_BUFFER_SECONDS) as usize
        {
            return;
        }
        let channels = self.output.channels();
        let sample_rate = self.output.sample_rate();
        loop {
            let Some((clip_index, source_index)) = self.timeline.audio_at_indices(self.position)
            else {
                let Some(next_start) = self.next_audio_start_after(self.position) else {
                    self.stop_decoder();
                    return;
                };
                self.position = next_start;
                continue;
            };
            let clip_id = self.timeline.clips()[clip_index].id.as_str();
            let clip_end = self.timeline.clips()[clip_index].timeline_end();
            let decoder_matches = self
                .decoder
                .as_ref()
                .is_some_and(|decoder| decoder.clip_id() == clip_id);
            if !decoder_matches {
                self.stop_decoder();
                self.decoder = {
                    let clip = &self.timeline.clips()[clip_index];
                    let source = &self.timeline.sources()[source_index];
                    AudioDecoder::open(clip, source, self.position, sample_rate, channels).ok()
                };
                if self.decoder.is_none() {
                    self.position = clip_end;
                    continue;
                }
            }
            let Some(decoder) = self.decoder.as_mut() else {
                return;
            };
            match decoder.next_samples() {
                Ok(Some(samples)) if !samples.is_empty() => {
                    let frames = samples.len() / channels.max(1);
                    self.output.push(&samples);
                    self.position += frames as f64 / sample_rate as f64;
                    if self.position >= clip_end {
                        self.position = clip_end;
                        self.stop_decoder();
                    }
                }
                Ok(Some(_)) => return,
                Ok(None) | Err(_) => {
                    self.stop_decoder();
                    self.position = clip_end;
                }
            }
            if self.output.queued_frames()
                >= (self.output.sample_rate() as f64 * AUDIO_BUFFER_SECONDS) as usize
            {
                return;
            }
        }
    }

    fn next_audio_start_after(&self, position: f64) -> Option<f64> {
        self.timeline
            .clips()
            .iter()
            .filter(|clip| {
                clip.track_kind == crate::ClipKind::Audio && clip.timeline_start > position
            })
            .map(|clip| clip.timeline_start)
            .min_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
    }

    fn stop_decoder(&mut self) {
        if let Some(mut decoder) = self.decoder.take() {
            decoder.stop();
        }
    }
}

struct AudioDecoder {
    input: Input,
    decoder: decoder::Audio,
    filter: filter::Graph,
    stream_index: usize,
    time_base: ffmpeg::Rational,
    source_time: f64,
    clip_id: String,
    eof: bool,
    flushed: bool,
}

impl AudioDecoder {
    fn open(
        clip: &PlaybackClip,
        source: &PlaybackSource,
        timeline_time: f64,
        sample_rate: u32,
        channels: usize,
    ) -> Result<Self, PlaybackError> {
        libav::ensure_init().map_err(audio_error)?;
        let source_time = clip.source_time_at(timeline_time);
        let remaining = (clip.source_out - source_time).max(0.0);
        if remaining <= 0.0 {
            return Err(PlaybackError::Audio(format!(
                "audio clip {} has no source time remaining",
                clip.id
            )));
        }
        let mut input = ffmpeg::format::input(&source.path)
            .map_err(|error| audio_error(libav::map_error(error)))?;
        let (stream_index, time_base) = {
            let stream = input
                .streams()
                .best(media::Type::Audio)
                .ok_or_else(|| PlaybackError::Audio("no audio stream".into()))?;
            (stream.index(), stream.time_base())
        };
        let decoder = {
            let stream = input
                .streams()
                .best(media::Type::Audio)
                .ok_or_else(|| PlaybackError::Audio("no audio stream".into()))?;
            let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
                .map_err(|error| audio_error(libav::map_error(error)))?;
            context
                .decoder()
                .audio()
                .map_err(|error| audio_error(libav::map_error(error)))?
        };
        libav::seek_to(&mut input, source_time);
        let filter = audio_filter_graph(&decoder, clip, remaining, sample_rate, channels)?;
        Ok(Self {
            input,
            decoder,
            filter,
            stream_index,
            time_base,
            source_time,
            clip_id: clip.id.clone(),
            eof: false,
            flushed: false,
        })
    }

    fn clip_id(&self) -> &str {
        &self.clip_id
    }

    fn next_samples(&mut self) -> Result<Option<Vec<f32>>, PlaybackError> {
        loop {
            if let Some(samples) = self.pull_filtered()? {
                if samples.is_empty() {
                    continue;
                }
                return Ok(Some(samples));
            }
            if self.eof && self.flushed {
                return Ok(None);
            }
            self.pump()?;
        }
    }

    fn stop(&mut self) {
        self.eof = true;
        self.flushed = true;
    }

    fn pull_filtered(&mut self) -> Result<Option<Vec<f32>>, PlaybackError> {
        let mut filtered = ffmpeg::frame::Audio::empty();
        let result = {
            let mut sink = self
                .filter
                .get("out")
                .ok_or_else(|| PlaybackError::Audio("missing abuffersink".into()))?;
            sink.sink().frame(&mut filtered)
        };
        match result {
            Ok(()) => libav::packed_f32(&filtered).map(Some).map_err(audio_error),
            Err(error) if libav::is_again(&error) || libav::is_eof(&error) => Ok(None),
            Err(error) => Err(audio_error(libav::map_error(error))),
        }
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
            if let Some(mut src) = self.filter.get("in") {
                let _ = src.source().flush();
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
                Err(error) => return Err(audio_error(libav::map_error(error))),
            }
        }
    }

    fn drain_decoder(&mut self) -> Result<(), PlaybackError> {
        let mut decoded = ffmpeg::frame::Audio::empty();
        loop {
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => self.push_decoded(&decoded)?,
                Err(error) if libav::is_again(&error) || libav::is_eof(&error) => return Ok(()),
                Err(error) => return Err(audio_error(libav::map_error(error))),
            }
        }
    }

    fn push_decoded(&mut self, decoded: &ffmpeg::frame::Audio) -> Result<(), PlaybackError> {
        if let Some(ts) = decoded.timestamp() {
            if libav::seconds(ts, self.time_base) + 1e-4 < self.source_time {
                return Ok(());
            }
        }
        let mut src = self
            .filter
            .get("in")
            .ok_or_else(|| PlaybackError::Audio("missing abuffer source".into()))?;
        src.source()
            .add(decoded)
            .map_err(|error| audio_error(libav::map_error(error)))
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn audio_error(error: PlaybackError) -> PlaybackError {
    match error {
        PlaybackError::Ffmpeg(message) => PlaybackError::Audio(message),
        other => other,
    }
}

fn audio_filter_graph(
    decoder: &decoder::Audio,
    clip: &PlaybackClip,
    remaining: f64,
    sample_rate: u32,
    channels: usize,
) -> Result<filter::Graph, PlaybackError> {
    let time_base = decoder.time_base();
    let (tb_num, tb_den) = if time_base.denominator() == 0 {
        (1, decoder.rate().max(1) as i32)
    } else {
        (time_base.numerator(), time_base.denominator())
    };
    let sample_fmt = decoder.format().name();
    let layout = channel_layout_arg(decoder);
    let args = format!(
        "time_base={tb_num}/{tb_den}:sample_rate={}:sample_fmt={sample_fmt}:channel_layout={layout}",
        decoder.rate().max(1)
    );
    let mut spec_parts = vec![
        "asetpts=PTS-STARTPTS".to_string(),
        format!("atrim=duration={}", libav::format_number(remaining)),
    ];
    spec_parts.extend(atempo_filters(clip.speed));
    if (clip.volume - 1.0).abs() > f32::EPSILON {
        spec_parts.push(format!("volume={}", clip.volume));
    }
    spec_parts.push(format!(
        "aformat=sample_fmts=flt:channel_layouts={}:sample_rates={}",
        output_layout_name(channels),
        sample_rate.max(1)
    ));
    let spec = spec_parts.join(",");
    let mut graph = filter::Graph::new();
    let buffer = filter::find("abuffer")
        .ok_or_else(|| PlaybackError::Audio("libavfilter abuffer is unavailable".into()))?;
    let sink = filter::find("abuffersink")
        .ok_or_else(|| PlaybackError::Audio("libavfilter abuffersink is unavailable".into()))?;
    graph
        .add(&buffer, "in", &args)
        .map_err(|error| audio_error(libav::map_error(error)))?;
    graph
        .add(&sink, "out", "")
        .map_err(|error| audio_error(libav::map_error(error)))?;
    {
        let mut out = graph
            .get("out")
            .ok_or_else(|| PlaybackError::Audio("missing abuffersink".into()))?;
        out.set_sample_format(Sample::F32(ffmpeg::format::sample::Type::Packed));
        out.set_sample_rate(sample_rate.max(1));
        out.set_channel_layout(output_layout(channels));
    }
    graph
        .output("in", 0)
        .map_err(|error| audio_error(libav::map_error(error)))?
        .input("out", 0)
        .map_err(|error| audio_error(libav::map_error(error)))?
        .parse(&spec)
        .map_err(|error| audio_error(libav::map_error(error)))?;
    graph
        .validate()
        .map_err(|error| audio_error(libav::map_error(error)))?;
    Ok(graph)
}

fn channel_layout_arg(decoder: &decoder::Audio) -> String {
    let layout = decoder.channel_layout();
    if layout.is_empty() {
        output_layout_name(decoder.channels() as usize)
    } else {
        format!("0x{:x}", layout.bits())
    }
}

fn output_layout(channels: usize) -> ChannelLayout {
    match channels {
        1 => ChannelLayout::MONO,
        _ => ChannelLayout::STEREO,
    }
}

fn output_layout_name(channels: usize) -> String {
    match channels {
        1 => "mono".into(),
        _ => "stereo".into(),
    }
}

fn atempo_filters(speed: f64) -> Vec<String> {
    if !speed.is_finite() || speed <= 0.0 {
        return Vec::new();
    }
    let mut remaining = speed;
    let mut filters = Vec::new();
    while remaining > 100.0 {
        filters.push("atempo=100".into());
        remaining /= 100.0;
    }
    while remaining < 0.5 {
        filters.push("atempo=0.5".into());
        remaining /= 0.5;
    }
    if (remaining - 1.0).abs() > 1e-6 {
        filters.push(format!("atempo={}", libav::format_number(remaining)));
    }
    filters
}

#[cfg(feature = "native-audio")]
struct AudioBuffer {
    samples: Mutex<VecDeque<f32>>,
    max_samples: usize,
    volume: AtomicU32,
    muted: AtomicBool,
}

#[cfg(feature = "native-audio")]
impl AudioBuffer {
    fn new(sample_rate: u32, channels: usize) -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
            max_samples: ((sample_rate as f64 * AUDIO_BUFFER_SECONDS) as usize)
                .saturating_mul(channels.max(1)),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            muted: AtomicBool::new(false),
        }
    }

    fn push(&self, samples: &[f32]) {
        let mut queue = self
            .samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if samples.len() >= self.max_samples {
            queue.clear();
            queue.extend(samples[samples.len() - self.max_samples..].iter().copied());
            return;
        }
        let available = self.max_samples.saturating_sub(samples.len());
        while queue.len() > available {
            queue.pop_front();
        }
        queue.extend(samples.iter().copied());
    }

    #[allow(dead_code)]
    fn pop(&self) -> Option<f32> {
        self.samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
    }

    fn clear(&self) {
        self.samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    fn queued_frames(&self, channels: usize) -> usize {
        self.samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
            / channels.max(1)
    }

    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Acquire))
    }
}

struct NullAudioOutput {
    sample_rate: u32,
    channels: usize,
}

impl AudioOutput for NullAudioOutput {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn push(&self, _samples: &[f32]) {}

    fn queued_frames(&self) -> usize {
        // There is no sink to fill. Reporting a full bounded queue keeps the
        // monotonic-clock fallback from decoding an entire long source into
        // nowhere while still allowing play/seek/mute controls to work.
        usize::MAX
    }

    fn clear(&self) {}

    fn set_volume(&self, _volume: f32) {}

    fn set_muted(&self, _muted: bool) {}
}

#[cfg(feature = "native-audio")]
struct CpalAudioOutput {
    buffer: Arc<AudioBuffer>,
    sample_rate: u32,
    channels: usize,
    _stream: cpal::Stream,
}

#[cfg(feature = "native-audio")]
impl AudioOutput for CpalAudioOutput {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn push(&self, samples: &[f32]) {
        self.buffer.push(samples);
    }

    fn queued_frames(&self) -> usize {
        self.buffer.queued_frames(self.channels)
    }

    fn clear(&self) {
        self.buffer.clear();
    }

    fn set_volume(&self, volume: f32) {
        self.buffer
            .volume
            .store(volume.to_bits(), Ordering::Release);
    }

    fn set_muted(&self, muted: bool) {
        self.buffer.muted.store(muted, Ordering::Release);
    }
}

fn make_audio_output(_clock: AudioClock, _enabled: bool) -> Box<dyn AudioOutput> {
    #[cfg(feature = "native-audio")]
    if _enabled {
        if let Some(output) = CpalAudioOutput::try_new(_clock.clone()) {
            return Box::new(output);
        }
    }
    Box::new(NullAudioOutput {
        sample_rate: 48_000,
        channels: 2,
    })
}

#[cfg(feature = "native-audio")]
impl CpalAudioOutput {
    fn try_new(clock: AudioClock) -> Option<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let sample_rate = supported.sample_rate().0.max(1);
        let channels = supported.channels().max(1) as usize;
        let config = supported.config();
        let buffer = Arc::new(AudioBuffer::new(sample_rate, channels));
        let callback_buffer = Arc::clone(&buffer);
        let callback_clock = clock.clone();
        let error_callback = |error| eprintln!("kerf playback audio stream: {error}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [f32], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .ok()?,
            cpal::SampleFormat::I16 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .ok()?,
            cpal::SampleFormat::U16 => device
                .build_output_stream(
                    &config,
                    move |data: &mut [u16], _| {
                        write_output(data, channels, &callback_buffer, &callback_clock)
                    },
                    error_callback,
                    None,
                )
                .ok()?,
            _ => return None,
        };
        stream.play().ok()?;
        clock.set_device_clock(sample_rate);
        Some(Self {
            buffer,
            sample_rate,
            channels,
            _stream: stream,
        })
    }
}

#[cfg(feature = "native-audio")]
fn write_output<T>(data: &mut [T], channels: usize, buffer: &AudioBuffer, clock: &AudioClock)
where
    T: cpal::Sample + cpal::FromSample<f32>,
{
    if !clock.is_playing() {
        for sample in data {
            *sample = T::from_sample(0.0);
        }
        return;
    }
    let muted = buffer.muted.load(Ordering::Acquire);
    let volume = buffer.volume();
    {
        let mut queue = buffer
            .samples
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for sample in data.iter_mut() {
            let value = queue.pop_front().unwrap_or(0.0);
            *sample = T::from_sample(if muted { 0.0 } else { value * volume });
        }
    }
    clock.record_frames((data.len() / channels.max(1)) as u64);
}

#[cfg(test)]
mod tests {
    use super::{atempo_filters, AudioClock};
    use crate::ClockSource;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn monotonic_clock_advances_and_stops_at_pause() {
        let clock = AudioClock::new();
        clock.play_at(2.0);
        thread::sleep(Duration::from_millis(5));
        let playing_position = clock.now();
        assert!(playing_position >= 2.0);
        assert_eq!(clock.source(), ClockSource::MonotonicFallback);

        clock.pause();
        let paused_position = clock.now();
        thread::sleep(Duration::from_millis(5));
        assert!((clock.now() - paused_position).abs() < 0.01);
    }

    #[test]
    fn atempo_chain_keeps_each_filter_inside_ffmpeg_limits() {
        assert_eq!(atempo_filters(2.0), vec!["atempo=2"]);
        assert_eq!(atempo_filters(0.25), vec!["atempo=0.5", "atempo=0.5"]);
        assert_eq!(atempo_filters(250.0), vec!["atempo=100", "atempo=2.5"]);
    }
}
