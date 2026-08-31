//! Cut media engine. Probe, encode, compile, proxy, and YOLO tracking.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// Timeline compilation is kept in the lightweight `cut-core` crate. Re-export
// its modules from the engine crate so Rust consumers have one coherent API
// while media/vision changes do not rebuild the compiler.
pub use cut_core::{compile, compile_v1, overlay};
pub mod encoder;
pub mod enrich;
pub mod error;
pub mod ffmpeg;
pub mod probe;
pub mod proxy;
pub mod render;
pub mod resources;
pub mod vision;

pub use encoder::{
    encoder_args, parse_encoder_capabilities, select_video_encoder, EncoderCapabilities,
    EncoderCapability, EncoderSelection, HardwareEncoderFamily, SelectEncoderOptions, VideoCodec,
};
pub use error::{EngineError, Result};
pub use probe::{
    detect_scenes, extract_frames, info, parse_scene_times, parse_silencedetect, ProbeInfo,
    SceneRange, SilenceRange,
};
pub use proxy::{create_proxy_cache_key, proxy_ffmpeg_args};
pub use resources::{
    budget, budget_for_cpus, HardwareEncodePolicy, ResourceBudget, DEFAULT_TRACKING_FPS,
};
