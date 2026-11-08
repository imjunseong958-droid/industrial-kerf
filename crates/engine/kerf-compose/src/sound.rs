//! The sound of media files for effects that draw it (Audio Spectrum and
//! Audio Waveform): each file decoded once by libav to interleaved stereo
//! floats at [`RATE`], resampled by libavfilter as playback resamples.
//!
//! A decode is shared by every compositor that asks for the same file, and
//! one asked for while it runs is waited for rather than started again. The
//! most recent ones stay kept, up to [`KEPT_BYTES`], after the compositors
//! that used them go, so the compositor an edit builds finds them decoded.

use crate::footage::init;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::{filter, frame, media, ChannelLayout};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::SystemTime;

/// The rate sound is kept at.
pub(crate) const RATE: u32 = 48_000;

/// How much decoded sound stays kept once no compositor holds it. The most
/// recently used decode is kept even when it alone is larger.
const KEPT_BYTES: usize = 512 << 20;

/// How many decodes stay kept once no compositor holds them.
const KEPT: usize = 32;

/// A file's sound once decoded; `None` for a file with none, or one that
/// could not be read.
type Decoded = OnceLock<Option<Arc<Vec<f32>>>>;

/// A file as it was when decoded.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    path: String,
    len: u64,
    modified: Option<SystemTime>,
}

impl Key {
    fn of(path: &str) -> Self {
        let metadata = std::fs::metadata(path).ok();
        Self {
            path: path.to_owned(),
            len: metadata.as_ref().map_or(0, |m| m.len()),
            modified: metadata.and_then(|m| m.modified().ok()),
        }
    }
}

/// The sound a compositor has asked for, by file.
#[derive(Default)]
pub(crate) struct Sounds {
    decoded: Mutex<HashMap<String, Arc<Decoded>>>,
}

impl Sounds {
    /// The sound of `path`, decoded the first time any compositor asks for it.
    pub fn of(&self, path: &str) -> Option<Arc<Vec<f32>>> {
        let (decoded, first) = {
            let mut decoded = self.decoded.lock().unwrap_or_else(PoisonError::into_inner);
            match decoded.get(path) {
                Some(found) => (found.clone(), None),
                None => {
                    let key = Key::of(path);
                    let found = shared()
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&key);
                    decoded.insert(path.to_owned(), found.clone());
                    (found, Some(key))
                }
            }
        };
        let sound = decoded
            .get_or_init(|| decode(path).ok().map(Arc::new))
            .clone();
        if let Some(key) = first {
            shared()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .used(key, decoded);
        }
        sound
    }
}

/// Decodes shared between compositors.
#[derive(Default)]
struct Shared {
    /// Every decode a compositor still holds, or one still running.
    live: HashMap<Key, Weak<Decoded>>,
    /// The decodes kept after their compositors go, most recently used last.
    kept: VecDeque<(Key, Arc<Decoded>)>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(Default::default)
}

impl Shared {
    /// The decode of `key`: one a compositor holds or one kept, else a new
    /// one for the caller to run.
    fn get(&mut self, key: &Key) -> Arc<Decoded> {
        if let Some(alive) = self.live.get(key).and_then(Weak::upgrade) {
            return alive;
        }
        self.live.retain(|_, decoded| decoded.strong_count() > 0);
        let decoded = Arc::new(Decoded::new());
        self.live.insert(key.clone(), Arc::downgrade(&decoded));
        decoded
    }

    /// Keep `decoded` as the most recently used, letting go of the oldest
    /// beyond the budget.
    fn used(&mut self, key: Key, decoded: Arc<Decoded>) {
        self.kept.retain(|(kept, _)| *kept != key);
        self.kept.push_back((key, decoded));
        let bytes = |decoded: &Decoded| {
            decoded
                .get()
                .and_then(Option::as_ref)
                .map_or(0, |sound| sound.len() * size_of::<f32>())
        };
        let mut total: usize = self.kept.iter().map(|(_, decoded)| bytes(decoded)).sum();
        while self.kept.len() > 1 && (total > KEPT_BYTES || self.kept.len() > KEPT) {
            if let Some((_, oldest)) = self.kept.pop_front() {
                total -= bytes(&oldest);
            }
        }
    }
}

/// Every sample of `path`'s best sound stream, from its start.
fn decode(path: &str) -> Result<Vec<f32>, String> {
    init()?;
    let failed = |error: ffmpeg::Error| format!("{path}: {error}");
    let mut input = ffmpeg::format::input(&path).map_err(failed)?;
    let (index, parameters) = {
        let stream = input
            .streams()
            .best(media::Type::Audio)
            .ok_or_else(|| format!("{path}: no sound"))?;
        (stream.index(), stream.parameters())
    };
    let mut decoder = ffmpeg::codec::context::Context::from_parameters(parameters)
        .and_then(|context| context.decoder().audio())
        .map_err(failed)?;
    let layout = match decoder.channel_layout() {
        layout if layout.is_empty() => ChannelLayout::default(i32::from(decoder.channels())),
        layout => layout,
    };
    let args = format!(
        "time_base=1/{rate}:sample_rate={rate}:sample_fmt={}:channel_layout=0x{:x}",
        decoder.format().name(),
        layout.bits(),
        rate = decoder.rate()
    );
    let mut graph = filter::Graph::new();
    let buffer = filter::find("abuffer").ok_or_else(|| "libavfilter has no abuffer".to_owned())?;
    let sink =
        filter::find("abuffersink").ok_or_else(|| "libavfilter has no abuffersink".to_owned())?;
    graph.add(&buffer, "in", &args).map_err(failed)?;
    graph.add(&sink, "out", "").map_err(failed)?;
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| {
            parser.parse(&format!(
                "aresample={RATE},aformat=sample_fmts=flt:sample_rates={RATE}:channel_layouts=stereo"
            ))
        })
        .map_err(failed)?;
    graph.validate().map_err(failed)?;

    let mut samples = Vec::new();
    let mut decoded = frame::Audio::empty();
    // Frames are laid end to end, as playback lays them.
    let mut next = 0;
    let mut feed = |decoder: &mut ffmpeg::decoder::Audio,
                    graph: &mut filter::Graph,
                    samples: &mut Vec<f32>|
     -> Result<(), String> {
        while decoder.receive_frame(&mut decoded).is_ok() {
            decoded.set_pts(Some(next));
            next += decoded.samples() as i64;
            graph
                .get("in")
                .ok_or_else(|| "missing abuffer".to_owned())?
                .source()
                .add(&decoded)
                .map_err(failed)?;
            drain(graph, samples)?;
        }
        Ok(())
    };
    for (stream, packet) in input.packets() {
        if stream.index() == index && decoder.send_packet(&packet).is_ok() {
            feed(&mut decoder, &mut graph, &mut samples)?;
        }
    }
    let _ = decoder.send_eof();
    feed(&mut decoder, &mut graph, &mut samples)?;
    graph
        .get("in")
        .ok_or_else(|| "missing abuffer".to_owned())?
        .source()
        .flush()
        .map_err(failed)?;
    drain(&mut graph, &mut samples)?;
    Ok(samples)
}

/// Move what the graph has finished into `samples`.
fn drain(graph: &mut filter::Graph, samples: &mut Vec<f32>) -> Result<(), String> {
    let mut out = frame::Audio::empty();
    let mut sink = graph
        .get("out")
        .ok_or_else(|| "missing abuffersink".to_owned())?;
    while sink.sink().frame(&mut out).is_ok() {
        let bytes = &out.data(0)[..out.samples() * 2 * 4];
        samples.extend(
            bytes
                .as_chunks::<4>().0.iter()
                .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tenth of a second of a stereo 16-bit WAV tone at `path`.
    fn tone(path: &std::path::Path) {
        let samples: Vec<i16> = (0..4_800)
            .flat_map(|n| {
                let v = ((n as f32 * 0.05).sin() * 8_000.0) as i16;
                [v, v]
            })
            .collect();
        let data = (samples.len() * 2) as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, wav).unwrap();
    }

    #[test]
    fn compositors_share_one_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        tone(&path);
        let path = path.to_str().unwrap();
        let first = Sounds::default();
        let sounds: Vec<_> = std::thread::scope(|scope| {
            let drawers: Vec<_> = (0..3)
                .map(|_| scope.spawn(|| first.of(path).expect("decodes")))
                .collect();
            drawers.into_iter().map(|d| d.join().unwrap()).collect()
        });
        assert_eq!(sounds[0].len(), 2 * 4_800);
        assert!(sounds.iter().all(|s| Arc::ptr_eq(s, &sounds[0])));
        // the compositor an edit builds finds it decoded after the first goes
        drop(first);
        let next = Sounds::default().of(path).expect("decodes");
        assert!(Arc::ptr_eq(&next, &sounds[0]));
    }

    #[test]
    fn kept_decodes_are_bounded() {
        let mut shared = Shared::default();
        let key = |n: usize| Key {
            path: format!("{n}.wav"),
            len: 0,
            modified: None,
        };
        for n in 0..KEPT + 8 {
            let decoded = shared.get(&key(n));
            decoded.get_or_init(|| Some(Arc::new(vec![0.0; 16])));
            shared.used(key(n), decoded);
            // the first stays the most recently used
            let first = shared.get(&key(0));
            shared.used(key(0), first);
        }
        assert_eq!(shared.kept.len(), KEPT);
        assert!(shared.kept.iter().any(|(k, _)| *k == key(0)));
        assert!(!shared.kept.iter().any(|(k, _)| *k == key(1)));
        // a decode neither kept nor held is forgotten
        let _ = shared.get(&key(KEPT + 9));
        assert_eq!(shared.live.len(), KEPT + 1);
    }
}
