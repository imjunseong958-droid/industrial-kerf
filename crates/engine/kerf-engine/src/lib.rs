//! Kerf media engine. Probe, encode, compile, render, proxy, and transcription.

// Timeline compilation is kept in the lightweight `kerf-core` crate. Re-export
// its modules from the engine crate so Rust consumers have one coherent API
// while media changes do not rebuild the compiler.
pub use kerf_core::{overlay, program};
pub mod analysis;
pub mod error;
pub mod media;
pub mod process;
pub mod recipe;
pub mod render;

pub use analysis::loudness::{measure_file, measure_program, Loudness};
pub use analysis::qc::{picture_qc, PictureCheck, PictureQc, QcRange};
pub use analysis::transcription::{
    cached_transcripts, listen_cached, parse_whisper_json, transcribe_cached,
    transcription_available, TimeRange, Transcript, TranscriptCue, TranscriptWord,
    TranscriptionBackend, TranscriptionOptions, TranscriptionProgress, TranscriptionResult,
    TranscriptionStage, MAX_LISTEN_RANGE_SECONDS, MAX_TRANSCRIPTION_RANGE_SECONDS,
    TRANSCRIPTION_SAMPLE_RATE, TRANSCRIPTION_SCHEMA_VERSION,
};
pub use analysis::unheard::{unheard_speech, Unheard};
pub use error::{EngineError, Result};
pub use media::index::{
    cached_media_index, cached_media_sound, media_index, small_grey_frames, MediaIndex, Peak, Tempo,
};
pub use media::probe::{
    cached_exact_frames, cached_filmstrip, cached_waveform, cached_waveform_range,
    create_contact_sheet, create_labelled_contact_sheet, detect_scenes, extract_exact_frames,
    extract_frames, info, media_info, parse_scene_times, parse_silencedetect, FilmstripResult,
    FrameGrab, FrameGrabSet, MediaInfo, ProbeInfo, SceneRange, SilenceRange, WaveformBucket,
    WaveformResult,
};
pub use media::proxy::{
    cached_proxy, create_proxy_cache_key, make_proxy, proxy_ffmpeg_args, PROXY_WIDTH,
};
pub use process::resources::{budget, budget_for_cpus, HardwareEncodePolicy, ResourceBudget};
pub use process::sidecar::CancellationToken;
pub use render::encoder::{
    encoder_args, parse_encoder_capabilities, select_video_encoder, EncoderCapabilities,
    EncoderCapability, EncoderSelection, HardwareEncoderFamily, SelectEncoderOptions, VideoCodec,
};
