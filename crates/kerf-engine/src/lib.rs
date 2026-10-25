//! Kerf media engine. Probe, encode, compile, render, proxy, and transcription.

// Timeline compilation is kept in the lightweight `kerf-core` crate. Re-export
// its modules from the engine crate so Rust consumers have one coherent API
// while media changes do not rebuild the compiler.
pub use kerf_core::{overlay, program};
pub mod derivatives;
pub mod encoder;
pub mod error;
pub mod ffmpeg;
pub mod index;
pub mod probe;
pub mod proxy;
pub mod raster;
pub mod render;
pub mod resources;
pub mod sidecar;
pub mod still;
pub mod transcription;

pub use encoder::{
    encoder_args, parse_encoder_capabilities, select_video_encoder, EncoderCapabilities,
    EncoderCapability, EncoderSelection, HardwareEncoderFamily, SelectEncoderOptions, VideoCodec,
};
pub use error::{EngineError, Result};
pub use index::{media_index, MediaIndex, Peak, Tempo};
pub use probe::{
    cached_exact_frames, cached_filmstrip, cached_waveform, cached_waveform_range,
    create_contact_sheet, detect_scenes, extract_exact_frames, extract_frames, info,
    parse_scene_times, parse_silencedetect, FilmstripResult, FrameGrab, FrameGrabSet, ProbeInfo,
    SceneRange, SilenceRange, WaveformBucket, WaveformResult,
};
pub use proxy::{cached_proxy, create_proxy_cache_key, make_proxy, proxy_ffmpeg_args, PROXY_WIDTH};
pub use resources::{budget, budget_for_cpus, HardwareEncodePolicy, ResourceBudget};
pub use sidecar::CancellationToken;
pub use transcription::{
    cached_transcripts, listen_cached, parse_whisper_json, transcribe_cached, TimeRange,
    Transcript, TranscriptCue, TranscriptWord, TranscriptionBackend, TranscriptionOptions,
    TranscriptionProgress, TranscriptionResult, TranscriptionStage, MAX_LISTEN_RANGE_SECONDS,
    MAX_TRANSCRIPTION_RANGE_SECONDS, TRANSCRIPTION_SCHEMA_VERSION,
};
