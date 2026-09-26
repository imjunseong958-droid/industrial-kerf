//! Cut media engine. Probe, encode, compile, proxy, and region planning.

// Timeline compilation is kept in the lightweight `cut-core` crate. Re-export
// its modules from the engine crate so Rust consumers have one coherent API
// while media/vision changes do not rebuild the compiler.
pub use cut_core::{check, compile, compile_v1, overlay};
pub mod derivatives;
pub mod encoder;
pub mod enrich;
pub mod error;
pub mod ffmpeg;
pub mod probe;
pub mod proxy;
pub mod render;
pub mod resources;
pub mod transcription;
pub mod vision;

pub use encoder::{
    encoder_args, parse_encoder_capabilities, select_video_encoder, EncoderCapabilities,
    EncoderCapability, EncoderSelection, HardwareEncoderFamily, SelectEncoderOptions, VideoCodec,
};
pub use error::{EngineError, Result};
pub use probe::{
    cached_exact_frames, cached_filmstrip, cached_waveform, cached_waveform_range,
    create_contact_sheet, detect_scenes, extract_exact_frames, extract_frames, info,
    parse_scene_times, parse_silencedetect, FilmstripResult, FrameGrab, FrameGrabSet, ProbeInfo,
    SceneRange, SilenceRange, WaveformBucket, WaveformResult,
};
pub use proxy::{create_proxy_cache_key, proxy_ffmpeg_args};
pub use resources::{budget, budget_for_cpus, HardwareEncodePolicy, ResourceBudget};
pub use transcription::{
    listen_cached, parse_whisper_json, transcribe_cached, CancellationToken, TimeRange, Transcript,
    TranscriptCue, TranscriptWord, TranscriptionBackend, TranscriptionOptions,
    TranscriptionProgress, TranscriptionResult, TranscriptionStage, MAX_LISTEN_RANGE_SECONDS,
    MAX_TRANSCRIPTION_RANGE_SECONDS, TRANSCRIPTION_SCHEMA_VERSION,
};
