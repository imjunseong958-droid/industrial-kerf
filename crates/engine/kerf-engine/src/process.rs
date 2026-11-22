//! The programs the engine runs beside itself, and the cores they share.
//!
//! [`ffmpeg`] finds FFmpeg and ffprobe and runs them. [`sidecar`] runs any such program, FFmpeg,
//! whisper, or a model's adapter, under supervision: cancelling it ends whatever it started, and
//! what it writes to stderr is read as it goes. [`resources`] is how many cores each kind of work
//! is given, so that a laptop is not asked for more than it has.

pub mod ffmpeg;
pub mod resources;
pub mod sidecar;
