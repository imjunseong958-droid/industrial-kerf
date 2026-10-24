//! The sound of media files for effects that draw it (Audio Spectrum and
//! Audio Waveform): each file decoded once by libav to interleaved stereo
//! floats at [`RATE`], resampled by libavfilter as playback resamples, and
//! kept while the compositor lives.

use crate::footage::init;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::{filter, frame, media, ChannelLayout};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// The rate sound is kept at.
pub(crate) const RATE: u32 = 48_000;

/// Decoded sound by file; `None` for a file with none, or one that could
/// not be read.
#[derive(Default)]
pub(crate) struct Sounds {
    decoded: Mutex<HashMap<String, Option<Arc<Vec<f32>>>>>,
}

impl Sounds {
    /// The sound of `path`, decoded the first time it is asked for.
    pub fn of(&self, path: &str) -> Option<Arc<Vec<f32>>> {
        let lock = || self.decoded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = lock().get(path) {
            return found.clone();
        }
        let decoded = decode(path).ok().map(Arc::new);
        lock().insert(path.to_owned(), decoded.clone());
        decoded
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
                .chunks_exact(4)
                .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
        );
    }
    Ok(())
}
